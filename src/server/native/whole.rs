//! The opt-in native whole-file upload.
//!
//! Each worker holds one connection and sends one file at a time. The file
//! is read once in packet-sized pieces, hashed in the order it is read, and
//! written through a sliding window of requests, so the link never idles
//! between pieces. The digest therefore comes from the send, and the summary
//! does not read the file again.
//!
//! The published file is never a partial file: the bytes go to
//! `<name>.uksfta-part`, the length is checked, and a rename puts the file
//! under its final name. A failure leaves the temp file behind, and the next
//! run truncates it or removes it as an orphan.

use openssh_sftp_client::file::File;
use openssh_sftp_client::Sftp;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::task::JoinSet;

use super::secret::Secret;
use super::{connect, publish, remote_dir, writer, NATIVE_BUFFER};
use crate::error::UksftaError;
use crate::server::config::Settings;
use crate::server::limit::RateLimiter;
use crate::server::progress::UploadProgress;
use crate::server::report::{plan_workers, UploadUnit};

/// The most unsent bytes the whole transfer holds across all its workers.
pub const WINDOW_BYTES: u64 = 64 * 1024 * 1024;
/// The runtime threads for one batch. More connections than threads is fine,
/// because a connection waits on the link, not on the processor.
const MAX_THREADS: u8 = 8;

/// What one whole-file send produced.
pub struct Sent {
    /// SHA-256 of the bytes that were read and written.
    pub hash: String,
    /// The bytes read from the local file.
    pub bytes: u64,
    /// The time from the first read to the last acknowledged write.
    pub elapsed: Duration,
}

/// One file that reached the server over the whole-file path.
pub struct SentFile {
    pub unit: UploadUnit,
    pub hash: String,
    pub seconds: f64,
}

/// The result of one batch over every worker.
#[derive(Default)]
pub struct Outcome {
    pub sent: Vec<SentFile>,
    /// One line per file that did not reach the server.
    pub errors: Vec<String>,
}

/// The unsent bytes one worker may hold for a file of `size` bytes.
///
/// The window is `min(requests × packet, 64 MiB, size)`, split by the number
/// of workers, but never less than one packet. `packet` is the write size:
/// the larger of `--buffer` and 261120 bytes.
pub fn window_bytes(requests: u16, packet: u32, size: u64, workers: usize) -> usize {
    let packet = usize::try_from(packet.max(4096)).unwrap_or(4096);
    let requested = usize::from(requests.max(1)).saturating_mul(packet);
    let file = usize::try_from(size.max(packet as u64)).unwrap_or(usize::MAX);
    let total = requested
        .min(usize::try_from(WINDOW_BYTES).unwrap_or(usize::MAX))
        .min(file);
    (total / workers.max(1)).max(packet)
}

/// The pieces one worker keeps in flight for a file of `size` bytes.
pub fn window_packets(requests: u16, packet: u32, size: u64, workers: usize) -> usize {
    let packet = usize::try_from(packet.max(4096)).unwrap_or(4096);
    (window_bytes(requests, packet as u32, size, workers) / packet).max(1)
}

/// Send every unit over its own native connection, one file at a time.
///
/// # Errors
/// Returns `Io` when the runtime cannot start. A connection or file failure
/// is reported per file in [`Outcome::errors`], so the other files continue.
pub fn run_batch(
    settings: &Settings,
    units: Vec<UploadUnit>,
    jobs: u8,
    password: Option<Arc<Secret>>,
    view: Option<Arc<UploadProgress>>,
) -> Result<Outcome, UksftaError> {
    let threads = usize::from(jobs.clamp(1, MAX_THREADS));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .map_err(UksftaError::Io)?;
    Ok(runtime.block_on(async move {
        let run_id = writer::run_id();
        let limiter = settings.transfer.limit.map(RateLimiter::new).map(Arc::new);
        let buckets = plan_workers(units, jobs);
        let workers = buckets.len().max(1);
        let mut tasks = JoinSet::new();
        for bucket in buckets {
            let settings = settings.clone();
            let run_id = run_id.clone();
            let password = password.clone();
            let limiter = limiter.clone();
            let view = view.clone();
            tasks.spawn(async move {
                one_worker(settings, run_id, bucket, password, limiter, view, workers).await
            });
        }

        let mut outcome = Outcome::default();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok((sent, mut errors)) => {
                    outcome.sent.extend(sent);
                    outcome.errors.append(&mut errors);
                }
                Err(error) => outcome
                    .errors
                    .push(format!("a whole-file worker stopped: {error}")),
            }
        }
        outcome
    }))
}

/// One worker: one connection, one file at a time.
async fn one_worker(
    settings: Settings,
    run_id: String,
    units: Vec<UploadUnit>,
    password: Option<Arc<Secret>>,
    limiter: Option<Arc<RateLimiter>>,
    view: Option<Arc<UploadProgress>>,
    workers: usize,
) -> (Vec<SentFile>, Vec<String>) {
    let session = match connect::connect_with(&settings, password.as_deref()).await {
        Ok(session) => session,
        Err(error) => {
            return (
                Vec::new(),
                vec![format!("the native whole-file connection failed: {error}")],
            );
        }
    };

    let mut sent = Vec::new();
    let mut errors = Vec::new();
    // One sweep per directory, not per file. A temp from a killed run is
    // removed when it is a day old; a fresh one is truncated by the send.
    let mut cleaned: HashSet<String> = HashSet::new();
    for unit in &units {
        let directory = remote_dir(&unit.remote).to_string();
        if !cleaned.insert(directory.clone()) {
            continue;
        }
        if let Err(error) = writer::clean_orphans(&session.sftp, &directory, None).await {
            eprintln!("Warning: cannot sweep {directory}: {error}");
        }
    }
    for unit in units {
        if let Some(view) = &view {
            view.file_started(&unit.remote);
        }
        let mut advance = |bytes: u64| {
            if let Some(view) = &view {
                view.advance(&unit.remote, bytes);
            }
        };
        let result = send_file(
            &session.sftp,
            &unit,
            &run_id,
            Params {
                packet: settings.transfer.buffer.max(NATIVE_BUFFER),
                requests: settings.transfer.requests,
                workers,
                preallocate: settings.segment.preallocate,
            },
            limiter.clone(),
            &mut advance,
        )
        .await;
        match result {
            Ok(done) => {
                if let Some(view) = &view {
                    view.file_done(&unit.remote, Some(&done.hash));
                }
                sent.push(SentFile {
                    unit,
                    hash: done.hash,
                    seconds: done.elapsed.as_secs_f64(),
                });
            }
            Err(error) => {
                if let Some(view) = &view {
                    view.file_failed(&unit.remote);
                }
                errors.push(format!("{}: {error}", unit.remote));
            }
        }
    }
    (sent, errors)
}

/// The transport parameters of one whole-file send.
#[derive(Debug, Clone, Copy)]
pub struct Params {
    /// The write size: the larger of `--buffer` and 261120 bytes.
    pub packet: u32,
    /// The `--requests` value, which bounds the window.
    pub requests: u16,
    /// The workers this batch runs, which splits the window.
    pub workers: usize,
    pub preallocate: bool,
}

/// Send one file: create the temp, hash every piece as it is written, check
/// the length and publish with a rename.
///
/// # Errors
/// Returns `Input` when the local file cannot be read, the server rejects a
/// write, the remote length differs, or the publish fails. A failed send
/// leaves the temp file behind and never the final name.
pub async fn send_file(
    sftp: &Sftp,
    unit: &UploadUnit,
    run_id: &str,
    params: Params,
    limiter: Option<Arc<RateLimiter>>,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<Sent, UksftaError> {
    let directory = remote_dir(&unit.remote);
    ensure_dir(sftp, directory).await?;

    let temp = writer::plan_temp_name(&unit.remote);
    writer::assert_not_loadable(&temp)?;
    writer::create_temp(sftp, &temp).await?;

    let size = std::fs::metadata(&unit.local)
        .map(|meta| meta.len())
        .map_err(|error| {
            UksftaError::Input(format!("cannot read {}: {error}", unit.local.display()))
        })?;
    if params.preallocate && size > 0 {
        match writer::preallocate(sftp, &temp, size).await? {
            writer::Preallocated::Full => {}
            writer::Preallocated::Unsupported => {
                eprintln!(
                    "Warning: the server cannot preallocate the temp file; the upload continues"
                );
            }
        }
    }

    let window = window_packets(params.requests, params.packet, size, params.workers);
    let file = writer::open_range_writer(sftp, &temp).await?;
    let mut sink = ConnectionSink {
        file: &file,
        limiter,
    };
    let sent = send_pieces(
        &unit.local,
        params.packet.max(NATIVE_BUFFER),
        window,
        &mut sink,
        progress,
    )
    .await?;
    file.close().await.map_err(writer::sftp_error)?;

    writer::check_len(sftp, &temp, sent.bytes).await?;
    publish::publish(sftp, &temp, &unit.remote, run_id, local_mtime(&unit.local)).await?;
    Ok(sent)
}

/// One write in flight. It owns its bytes and its handle clone.
type PieceFuture = Pin<Box<dyn std::future::Future<Output = Result<u64, UksftaError>> + Send>>;
/// Where a read piece goes.
trait PieceSink {
    fn piece(&mut self, offset: u64, data: Arc<Vec<u8>>) -> PieceFuture;
}

/// The connection's file handle, cloned for each piece so every write has
/// its own offset and the window never waits on a shared handle.
struct ConnectionSink<'a> {
    file: &'a File,
    limiter: Option<Arc<RateLimiter>>,
}

impl PieceSink for ConnectionSink<'_> {
    fn piece(&mut self, offset: u64, data: Arc<Vec<u8>>) -> PieceFuture {
        let mut file = self.file.clone();
        let limiter = self.limiter.clone();
        Box::pin(async move {
            file.seek(SeekFrom::Start(offset))
                .await
                .map_err(UksftaError::Io)?;
            if let Some(limiter) = limiter {
                limiter.wait(data.len() as u64).await;
            }
            let mut written = 0;
            while written < data.len() {
                let count = file
                    .write(&data[written..])
                    .await
                    .map_err(writer::sftp_error)?;
                if count == 0 {
                    return Err(UksftaError::Input(
                        "the server accepted no bytes".to_string(),
                    ));
                }
                written += count;
            }
            Ok(data.len() as u64)
        })
    }
}

/// Hashes the pieces on a thread of its own, so the send loop never waits
/// on SHA-256. The channel is small, so the thread cannot lag far behind
/// the read and the memory stays bounded.
struct Hasher {
    sender: std::sync::mpsc::SyncSender<Arc<Vec<u8>>>,
    handle: std::thread::JoinHandle<String>,
}

impl Hasher {
    fn start() -> Self {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Arc<Vec<u8>>>(4);
        let handle = std::thread::spawn(move || {
            let mut hasher = Sha256::new();
            for data in receiver {
                hasher.update(&data[..]);
            }
            let digest = hasher.finalize();
            digest.iter().map(|byte| format!("{byte:02x}")).collect()
        });
        Self { sender, handle }
    }

    fn update(&self, data: &Arc<Vec<u8>>) -> Result<(), UksftaError> {
        self.sender
            .send(Arc::clone(data))
            .map_err(|_| UksftaError::Input("the whole-file hash thread stopped".to_string()))
    }

    fn finish(self) -> Result<String, UksftaError> {
        drop(self.sender);
        self.handle
            .join()
            .map_err(|_| UksftaError::Input("the whole-file hash thread panicked".to_string()))
    }
}

/// Read the file once in pieces, hash them in order, and keep `window`
/// writes in flight. Every piece runs as its own task, so all of them make
/// progress; when the window is full the loop waits for one to finish
/// before reading the next piece.
async fn send_pieces<S: PieceSink>(
    path: &Path,
    packet: u32,
    window: usize,
    sink: &mut S,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<Sent, UksftaError> {
    let started = Instant::now();
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|error| UksftaError::Input(format!("cannot read {}: {error}", path.display())))?;
    let hasher = Hasher::start();
    let mut offset = 0u64;
    let window = window.max(1);
    let mut in_flight: JoinSet<Result<u64, UksftaError>> = JoinSet::new();
    loop {
        let Some(data) = read_piece(&mut file, path, packet).await? else {
            break;
        };
        let data = Arc::new(data);
        hasher.update(&data)?;
        if in_flight.len() >= window {
            progress(join_piece(&mut in_flight).await?);
        }
        let len = data.len() as u64;
        in_flight.spawn(sink.piece(offset, data));
        offset += len;
    }
    while !in_flight.is_empty() {
        progress(join_piece(&mut in_flight).await?);
    }
    Ok(Sent {
        hash: hasher.finish()?,
        bytes: offset,
        elapsed: started.elapsed(),
    })
}

/// Wait for one piece write to finish and return its bytes.
async fn join_piece(in_flight: &mut JoinSet<Result<u64, UksftaError>>) -> Result<u64, UksftaError> {
    match in_flight.join_next().await {
        Some(Ok(result)) => result,
        Some(Err(error)) => Err(UksftaError::Input(format!(
            "a whole-file write stopped: {error}"
        ))),
        None => Err(UksftaError::Input(
            "the whole-file window is empty".to_string(),
        )),
    }
}

/// The next piece, or `None` at the end of the file.
async fn read_piece(
    file: &mut tokio::fs::File,
    path: &Path,
    packet: u32,
) -> Result<Option<Vec<u8>>, UksftaError> {
    let mut data = vec![0u8; usize::try_from(packet.max(1)).unwrap_or(4096)];
    let mut filled = 0usize;
    while filled < data.len() {
        let read = file.read(&mut data[filled..]).await.map_err(|error| {
            UksftaError::Input(format!("cannot read {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    if filled == 0 {
        return Ok(None);
    }
    data.truncate(filled);
    Ok(Some(data))
}

/// Create `path` and its parents on the server when they are absent. A
/// second worker may create the same directory between the check and the
/// create, so the create result is checked against the directory itself.
async fn ensure_dir(sftp: &Sftp, path: &str) -> Result<(), UksftaError> {
    if path.is_empty() || path == "." {
        return Ok(());
    }
    let mut fs = sftp.fs();
    if fs.metadata(path).await.is_ok() {
        return Ok(());
    }
    let mut prefix = String::new();
    for part in path.split('/').filter(|part| !part.is_empty()) {
        prefix.push('/');
        prefix.push_str(part);
        if fs.metadata(&prefix).await.is_ok() {
            continue;
        }
        if let Err(error) = fs.create_dir(&prefix).await {
            if fs.metadata(&prefix).await.is_err() {
                return Err(UksftaError::Input(format!(
                    "cannot create the remote directory {prefix}: {error}"
                )));
            }
        }
    }
    Ok(())
}

/// The modification time the server keeps, in seconds since the epoch.
fn local_mtime(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn local_file(dir: &tempfile::TempDir, name: &str, size: u64) -> PathBuf {
        use std::io::Write;
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).expect("create the local file");
        let block: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();
        let mut left = size;
        while left > 0 {
            let want = usize::try_from(left.min(block.len() as u64)).expect("block");
            file.write_all(&block[..want])
                .expect("write the local file");
            left -= want as u64;
        }
        path
    }

    /// A sink that keeps every piece and fails on a gap.
    struct MemorySink {
        data: Vec<u8>,
    }

    impl PieceSink for MemorySink {
        fn piece(&mut self, offset: u64, data: Arc<Vec<u8>>) -> PieceFuture {
            assert_eq!(offset, self.data.len() as u64, "the pieces follow on");
            let len = data.len() as u64;
            self.data.extend_from_slice(&data);
            Box::pin(async move { Ok(len) })
        }
    }

    #[test]
    fn the_window_follows_the_requests_packet_and_workers() {
        let packet = 261_120u32;
        assert_eq!(window_packets(256, packet, 15_460_000_000, 8), 32);
        assert_eq!(window_packets(256, packet, 15_460_000_000, 1), 256);
        assert_eq!(window_packets(64, packet, 1 << 30, 1), 64);
        assert_eq!(window_packets(1, packet, 1 << 30, 1), 1);
        assert_eq!(
            window_packets(256, packet, 1_000, 8),
            1,
            "a small file takes one packet"
        );
        assert!(
            window_bytes(256, packet, u64::MAX, 1) <= usize::try_from(WINDOW_BYTES).unwrap(),
            "one worker never holds more than 64 MiB"
        );
    }

    #[tokio::test]
    async fn the_send_digest_equals_the_file_digest() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        for size in [0u64, 1, 4095, 4096, 65_537, 1_048_577] {
            let path = local_file(&dir, &format!("f-{size}.pbo"), size);
            let expected = crate::util::file_sha256(&path).expect("the local digest");
            for packet in [4096u32, 65_536, 261_120] {
                for window in [1usize, 2, 4, 32] {
                    let mut sink = MemorySink { data: Vec::new() };
                    let mut noop = |_bytes: u64| {};
                    let sent = send_pieces(&path, packet, window, &mut sink, &mut noop)
                        .await
                        .expect("the send loop");
                    assert_eq!(
                        sent.hash, expected,
                        "{size} bytes in {packet}-byte pieces, window {window}"
                    );
                    assert_eq!(sent.bytes, size);
                    assert_eq!(
                        sink.data,
                        std::fs::read(&path).expect("the local file"),
                        "{size} bytes in {packet}-byte pieces, window {window}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn an_empty_file_hashes_to_the_empty_digest() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = local_file(&dir, "empty.pbo", 0);
        let mut sink = MemorySink { data: Vec::new() };
        let mut noop = |_bytes: u64| {};
        let sent = send_pieces(&path, 261_120, 4, &mut sink, &mut noop)
            .await
            .expect("the send loop");
        assert_eq!(
            sent.hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(sent.bytes, 0);
        assert!(sink.data.is_empty());
    }
}
