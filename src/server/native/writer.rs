//! The temp file, its write-only handles, range writes and the run lock.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use openssh_sftp_client::file::File;
use openssh_sftp_client::Sftp;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

use super::segment::Chunk;
use crate::error::UksftaError;

/// The temp name. A later run recognises its own file, so no run id.
pub fn plan_temp_name(final_path: &str) -> String {
    format!("{final_path}.uksfta-part")
}

/// The name the old final file is moved to during the fallback publish.
pub fn plan_aside_name(final_path: &str, run_id: &str) -> String {
    format!("{final_path}.uksfta-old-{run_id}")
}

/// Refuse a temp name that Arma would load.
///
/// # Errors
/// Returns `Input` for a `.pbo`, `.bisign` or `.bikey` ending.
pub fn assert_not_loadable(name: &str) -> Result<(), UksftaError> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".pbo") || lower.ends_with(".bisign") || lower.ends_with(".bikey") {
        return Err(UksftaError::Input(format!(
            "refusing a temporary name that Arma would load: {name}"
        )));
    }
    Ok(())
}

/// A run id of 16 lowercase hex characters.
pub fn run_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let mixed = (nanos as u64) ^ ((u64::from(std::process::id())) << 48);
    format!("{mixed:016x}")
}

/// An exclusive run lock for one pack.
#[derive(Debug)]
pub struct RunLock {
    path: PathBuf,
}

impl Drop for RunLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Take the lock under `.uksfta/upload/<pack_id>/`. A lock older than six
/// hours is removed with one warning and retaken.
///
/// # Errors
/// Returns `Input` when another run holds a fresh lock.
pub fn take_run_lock(state_root: &Path, pack_id: &str) -> Result<RunLock, UksftaError> {
    let dir = state_root.join(".uksfta").join("upload").join(pack_id);
    fs::create_dir_all(&dir).map_err(UksftaError::Io)?;
    let path = dir.join("run.lock");

    match create_lock(&path) {
        Ok(()) => Ok(RunLock { path }),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if !lock_is_stale(&path) {
                return Err(UksftaError::Locked(format!(
                    "another upload is running for this pack {pack_id}; no change was made"
                )));
            }
            eprintln!(
                "Warning: removing a stale upload lock at {}",
                path.display()
            );
            fs::remove_file(&path).map_err(UksftaError::Io)?;
            create_lock(&path).map_err(UksftaError::Io)?;
            Ok(RunLock { path })
        }
        Err(error) => Err(UksftaError::Io(error)),
    }
}

/// A lock is stale when its owner is gone or it is older than six hours.
fn lock_is_stale(path: &Path) -> bool {
    if let Some(pid) = lock_pid(path) {
        if pid != i64::from(std::process::id()) && !process_alive(pid) {
            return true;
        }
    }
    fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > Duration::from_secs(6 * 60 * 60))
}

/// The process id the lock file records, when it is readable.
fn lock_pid(path: &Path) -> Option<i64> {
    let text = fs::read_to_string(path).ok()?;
    text.split_whitespace().next()?.parse().ok()
}

#[cfg(unix)]
fn process_alive(pid: i64) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 checks for existence and sends nothing.
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn process_alive(_pid: i64) -> bool {
    true
}

fn create_lock(path: &Path) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    writeln!(file, "{} {stamp}", std::process::id())?;
    Ok(())
}

/// Create and truncate the temp file once.
///
/// The file is empty and carries no data to lose, so it is not synced. The
/// first checkpoint write and the final sync before publish cover durability.
///
/// # Errors
/// Returns `Input` when the server rejects the open or the truncate.
pub async fn create_temp(sftp: &Sftp, path: &str) -> Result<(), UksftaError> {
    assert_not_loadable(path)?;
    let mut file = sftp
        .options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .await
        .map_err(sftp_error)?;
    file.set_len(0).await.map_err(sftp_error)?;
    file.close().await.map_err(sftp_error)?;
    Ok(())
}

/// What preallocation achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preallocated {
    /// The temp file now has its final length.
    Full,
    /// The server does not support the request. The upload carries on.
    Unsupported,
}

/// Set the temp file to its final length before the first range, so a
/// shortfall shows before the transfer starts. This client exposes no
/// `fallocate` extension, so the length comes from `set_len`.
///
/// # Errors
/// Returns `Input` for a real failure, such as no space.
pub async fn preallocate(sftp: &Sftp, path: &str, size: u64) -> Result<Preallocated, UksftaError> {
    let mut file = sftp
        .options()
        .write(true)
        .create(false)
        .truncate(false)
        .open(path)
        .await
        .map_err(sftp_error)?;
    let outcome = match file.set_len(size).await {
        Ok(()) => Ok(Preallocated::Full),
        Err(openssh_sftp_client::Error::SftpError(
            openssh_sftp_client::error::SftpErrorKind::OpUnsupported,
            _,
        )) => Ok(Preallocated::Unsupported),
        Err(error) => Err(UksftaError::Input(format!(
            "cannot preallocate {path} to {size} bytes: {error}"
        ))),
    };
    let _ = file.close().await;
    outcome
}

/// Open one write-only handle at the temp path. A missing file is refused.
///
/// # Errors
/// Returns `Input` when the server rejects the open.
pub async fn open_range_writer(sftp: &Sftp, path: &str) -> Result<File, UksftaError> {
    let file = sftp
        .options()
        .write(true)
        .create(false)
        .truncate(false)
        .open(path)
        .await
        .map_err(sftp_error)?;
    Ok(file)
}

/// The most write handles one connection keeps in flight.
pub const PIPELINE_MAX: usize = 8;

/// The write handles to keep in flight on one connection, bounded by the
/// `-R` request count.
pub fn pipeline_depth(requests: u16) -> usize {
    usize::from(requests.max(1)).min(PIPELINE_MAX)
}

/// Split one range into at most `count` disjoint pieces, one per handle.
/// Each piece stays at least one request long, so a small range is sent as
/// one piece rather than as several short requests.
pub fn pieces(chunk: &Chunk, count: usize, buffer: u32) -> Vec<Chunk> {
    let total = u64::from(chunk.len);
    let request = u64::from(buffer.max(4096));
    let most = usize::try_from(total.div_ceil(request).max(1)).unwrap_or(1);
    let count = count.clamp(1, most);
    let per = total.div_ceil(u64::try_from(count).unwrap_or(1));

    let mut out = Vec::with_capacity(count);
    let mut start = 0u64;
    while start < total {
        let len = per.min(total - start);
        out.push(Chunk {
            index: u32::try_from(out.len()).unwrap_or(0),
            start: chunk.start + start,
            len: u32::try_from(len).unwrap_or(u32::MAX),
        });
        start += len;
    }
    out
}

/// The local file one piece reads from, with a buffer it reuses for every
/// piece it sends. A reader lives as long as a worker, so a range costs a
/// seek and reads, not an open and an allocation.
pub struct LocalReader {
    path: PathBuf,
    file: tokio::fs::File,
    buffer: Vec<u8>,
}

impl LocalReader {
    /// Open the local file once and size its read buffer.
    ///
    /// # Errors
    /// Returns `Input` when the file cannot be opened.
    pub async fn open(path: &Path, buffer: u32) -> Result<Self, UksftaError> {
        let file = tokio::fs::File::open(path).await.map_err(|error| {
            UksftaError::Input(format!("cannot read {}: {error}", path.display()))
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            buffer: vec![0u8; usize::try_from(buffer.max(4096)).unwrap_or(4096)],
        })
    }

    /// Move to the start of a range.
    ///
    /// # Errors
    /// Returns `Input` when the seek fails.
    pub async fn seek_to(&mut self, offset: u64) -> Result<(), UksftaError> {
        self.file
            .seek(SeekFrom::Start(offset))
            .await
            .map(drop)
            .map_err(|error| {
                UksftaError::Input(format!("cannot seek {}: {error}", self.path.display()))
            })
    }

    /// Read the next block into the buffer and count the bytes.
    ///
    /// # Errors
    /// Returns `Input` when the read fails or the file ended early.
    pub async fn fill(&mut self, remaining: u64) -> Result<usize, UksftaError> {
        let want =
            usize::try_from(remaining.min(self.buffer.len() as u64)).unwrap_or(self.buffer.len());
        let read = self
            .file
            .read(&mut self.buffer[..want])
            .await
            .map_err(|error| {
                UksftaError::Input(format!("cannot read {}: {error}", self.path.display()))
            })?;
        if read == 0 {
            return Err(UksftaError::Input(format!(
                "the local file ended early: {}",
                self.path.display()
            )));
        }
        Ok(read)
    }

    /// The bytes the last [`LocalReader::fill`] read.
    pub fn bytes(&self) -> &[u8] {
        &self.buffer
    }
}

/// The readers of one writer: one for each pipelined handle, because the
/// pieces of a range are read from different offsets at the same time.
pub struct LocalReaders {
    slots: Vec<LocalReader>,
}

impl LocalReaders {
    /// Open one reader for each handle.
    ///
    /// # Errors
    /// Returns `Input` when the local file cannot be opened.
    pub async fn open(path: &Path, buffer: u32, slots: usize) -> Result<Self, UksftaError> {
        let mut readers = Vec::with_capacity(slots.max(1));
        for _ in 0..slots.max(1) {
            readers.push(LocalReader::open(path, buffer).await?);
        }
        Ok(Self { slots: readers })
    }

    /// The number of readers.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.slots.len()
    }
}

/// The write handles of one connection.
///
/// The handles share one server file handle and keep a separate offset each,
/// so a range splits across them and several writes are in flight at once.
pub struct RangeWriter {
    handles: Vec<File>,
}

impl RangeWriter {
    /// Open `handles` write handles on one connection.
    ///
    /// # Errors
    /// Returns `Input` when the server rejects the open.
    pub async fn open(sftp: &Sftp, path: &str, handles: usize) -> Result<Self, UksftaError> {
        let base = open_range_writer(sftp, path).await?;
        let owned = (0..handles.max(1)).map(|_| base.clone()).collect();
        drop(base);
        Ok(Self { handles: owned })
    }

    /// Wrap one handle that is already open.
    #[cfg(test)]
    pub fn from_handle(file: File) -> Self {
        Self {
            handles: vec![file],
        }
    }

    /// The number of writes this writer can keep in flight.
    #[cfg(test)]
    pub fn depth(&self) -> usize {
        self.handles.len()
    }

    /// Write one range at its own offset, one piece per handle.
    ///
    /// # Errors
    /// Returns `Input` when the local file ends early, the server accepts no
    /// bytes, a handle has no reader, or either side reports an error. A
    /// failed range is sent again whole, so a partly written range is safe to
    /// retry.
    pub async fn write(
        &mut self,
        chunk: &Chunk,
        readers: &mut LocalReaders,
        buffer: u32,
        limiter: Option<&super::super::limit::RateLimiter>,
        progress: &mut (dyn FnMut(u64) + Send),
    ) -> Result<(), UksftaError> {
        let split = pieces(chunk, self.handles.len(), buffer);
        if readers.slots.len() < split.len() {
            return Err(UksftaError::Input(format!(
                "the writer holds {} piece(s) but its readers number {}",
                split.len(),
                readers.slots.len()
            )));
        }
        let progress = std::sync::Mutex::new(progress);

        let futures = self
            .handles
            .iter_mut()
            .zip(readers.slots.iter_mut())
            .zip(&split)
            .map(|((file, reader), piece)| write_piece(file, piece, reader, limiter, &progress))
            .collect::<Vec<_>>();

        for result in super::fanout::all(futures).await {
            result?;
        }
        Ok(())
    }

    /// Flush the file to the server's disk. Every write has been answered, so
    /// one handle is enough.
    ///
    /// # Errors
    /// Returns `Input` when the server refuses the sync.
    pub async fn sync_all(&mut self) -> Result<(), UksftaError> {
        match self.handles.first_mut() {
            Some(file) => file.sync_all().await.map_err(sftp_error),
            None => Ok(()),
        }
    }

    /// Close the handles. The last reference sends the close request.
    ///
    /// # Errors
    /// Returns `Input` when the server refuses the close.
    pub async fn close(self) -> Result<(), UksftaError> {
        match self.handles.into_iter().last() {
            Some(file) => file.close().await.map_err(sftp_error),
            None => Ok(()),
        }
    }
}

/// Write one piece from its own handle at the explicit offset.
async fn write_piece(
    file: &mut File,
    piece: &Chunk,
    reader: &mut LocalReader,
    limiter: Option<&super::super::limit::RateLimiter>,
    progress: &std::sync::Mutex<&mut (dyn FnMut(u64) + Send)>,
) -> Result<(), UksftaError> {
    file.seek(SeekFrom::Start(piece.start))
        .await
        .map_err(UksftaError::Io)?;
    reader.seek_to(piece.start).await?;

    let mut remaining = u64::from(piece.len);
    while remaining > 0 {
        let read = reader.fill(remaining).await?;
        if let Some(limiter) = limiter {
            limiter.wait(read as u64).await;
        }
        let mut written = 0;
        while written < read {
            let count = file
                .write(&reader.bytes()[written..read])
                .await
                .map_err(sftp_error)?;
            if count == 0 {
                return Err(UksftaError::Input(
                    "the server accepted no bytes".to_string(),
                ));
            }
            written += count;
        }
        remaining -= read as u64;
        {
            // The lock is held only for the callback, never across an await.
            let mut report = progress
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            report(read as u64);
        }
    }
    Ok(())
}

/// Sync the finished temp file and compare its length with the local file.
///
/// # Errors
/// Returns `Input` when the remote length differs.
pub async fn finish(sftp: &Sftp, path: &str, size: u64) -> Result<(), UksftaError> {
    let mut file = sftp
        .options()
        .write(true)
        .create(false)
        .truncate(false)
        .open(path)
        .await
        .map_err(sftp_error)?;
    if sftp.support_fsync() {
        file.sync_all().await.map_err(sftp_error)?;
    }
    file.close().await.map_err(sftp_error)?;

    let mut fs = sftp.fs();
    let metadata = fs.metadata(path).await.map_err(sftp_error)?;
    let len = metadata.len().unwrap_or(u64::MAX);
    if len != size {
        return Err(UksftaError::Input(format!(
            "the server holds {len} bytes, expected {size}"
        )));
    }
    Ok(())
}

fn sftp_error(error: openssh_sftp_client::Error) -> UksftaError {
    UksftaError::Input(format!("sftp: {error}"))
}

/// The largest number of orphan temps one run removes.
pub const MAX_ORPHANS: usize = 8;
/// The age after which an orphan temp is removed.
pub const ORPHAN_AGE_SECS: u64 = 24 * 60 * 60;

/// Remove stale temp and aside files from the pack directory. Only regular
/// files that carry a temp marker and are at least a day old are removed.
///
/// # Errors
/// Returns `Input` when the directory cannot be opened.
pub async fn clean_orphans(
    sftp: &Sftp,
    dir: &str,
    keep: Option<&str>,
) -> Result<Vec<String>, UksftaError> {
    use tokio_stream::StreamExt;

    let mut fs = sftp.fs();
    let listing = match fs.open_dir(dir).await {
        Ok(listing) => listing,
        Err(error) => {
            eprintln!("Warning: cannot list {dir} for orphan temps: {error}");
            return Ok(Vec::new());
        }
    };
    let entries = listing.read_dir();
    let mut entries = std::pin::pin!(entries);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let mut removed = Vec::new();
    while removed.len() < MAX_ORPHANS {
        let Some(entry) = entries.next().await else {
            break;
        };
        let Ok(entry) = entry else {
            continue;
        };
        let name = entry.filename().to_string_lossy().into_owned();
        if !is_temp_name(&name) || keep == Some(name.as_str()) {
            continue;
        }
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let metadata = entry.metadata();
        let Some(modified) = metadata
            .modified()
            .map(openssh_sftp_client::UnixTimeStamp::into_raw)
        else {
            continue;
        };
        if now.saturating_sub(u64::from(modified)) < ORPHAN_AGE_SECS {
            continue;
        }
        let path = join_name(dir, &name);
        if fs.remove_file(&path).await.is_ok() {
            eprintln!("Warning: removed the orphan temp {path}");
            removed.push(path);
        }
    }
    Ok(removed)
}

/// The names this command leaves behind between runs.
fn is_temp_name(name: &str) -> bool {
    name.contains(".uksfta-part-") || name.contains(".uksfta-old-")
}

/// Ask for the free space in the target directory. `None` means the server
/// cannot say, which is one warning, not a failure.
///
/// # Errors
/// Returns `Input` when the free space is known to be short of `need`.
pub async fn check_space(sftp: &Sftp, dir: &str, need: u64) -> Result<Option<u64>, UksftaError> {
    if !sftp.support_statvfs() {
        eprintln!("Warning: the server cannot report free space; the upload continues");
        return Ok(None);
    }
    let mut fs = sftp.fs();
    if fs.metadata(dir).await.is_err() {
        eprintln!("Warning: {dir} does not exist yet; the upload creates it");
        return Ok(None);
    }
    let stat = match fs.statvfs(dir).await {
        Ok(stat) => stat,
        Err(openssh_sftp_client::Error::SftpError(
            openssh_sftp_client::error::SftpErrorKind::OpUnsupported,
            _,
        )) => {
            eprintln!("Warning: the server cannot report free space; the upload continues");
            return Ok(None);
        }
        Err(error) => {
            return Err(UksftaError::Input(format!(
                "cannot ask for free space at {dir}: {error}"
            )))
        }
    };
    let free = stat.bavail.saturating_mul(stat.frsize);
    if stat.frsize == 0 || stat.blocks == 0 || free == 0 {
        eprintln!("Warning: the server reported no usable free-space data; the upload continues");
        return Ok(None);
    }
    if free < need {
        return Err(UksftaError::Input(format!(
            "the server holds {free} free bytes but the upload needs {need}"
        )));
    }
    Ok(Some(free))
}

fn join_name(dir: &str, name: &str) -> String {
    format!("{}/{}", dir.trim_end_matches('/'), name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_and_aside_names_are_stable() {
        assert_eq!(plan_temp_name("/r/a/b.pbo"), "/r/a/b.pbo.uksfta-part");
        assert_eq!(plan_temp_name("/r/a/b.pbo"), plan_temp_name("/r/a/b.pbo"));
        let aside = plan_aside_name("/r/a/b.pbo", "0123456789abcdef");
        assert!(aside.ends_with(".uksfta-old-0123456789abcdef"), "{aside}");
        assert!(plan_temp_name("/r/a/b.pbo").len() < 4096);
    }

    #[test]
    fn a_loadable_temp_name_is_refused() {
        for name in ["A.PBO", "a.bisign", "a.bikey"] {
            assert!(assert_not_loadable(name).is_err(), "{name}");
        }
        assert!(assert_not_loadable("a.pbo.uksfta-part").is_ok());
    }

    #[test]
    fn the_run_id_is_sixteen_hex_characters() {
        let id = run_id();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_second_run_lock_is_refused_and_drop_releases_it() {
        let dir = tempfile::TempDir::new().unwrap();
        let first = take_run_lock(dir.path(), "1").unwrap();
        let error = take_run_lock(dir.path(), "1").unwrap_err();
        assert!(error.to_string().contains("another upload"));
        drop(first);
        assert!(take_run_lock(dir.path(), "1").is_ok());
    }

    #[test]
    fn only_temp_markers_are_orphans() {
        assert!(is_temp_name("x.pbo.uksfta-part-old"));
        assert!(is_temp_name("x.pbo.uksfta-old-0123456789abcdef"));
        assert!(!is_temp_name("x.pbo.uksfta-part"), "the live temp is kept");
        assert!(!is_temp_name("x.pbo"));
        assert!(!is_temp_name("x.pbo.bisign"));
    }

    #[test]
    fn an_orphan_path_joins_the_directory_once() {
        assert_eq!(join_name("/srv/a", "x.part"), "/srv/a/x.part");
        assert_eq!(join_name("/srv/a/", "x.part"), "/srv/a/x.part");
    }

    #[test]
    fn the_pieces_cover_the_range_once_at_pinned_offsets() {
        let chunk = Chunk {
            index: 9,
            start: 4 * 1024 * 1024,
            len: 16 * 1024 * 1024,
        };
        for count in [1usize, 2, 3, 4, 8] {
            let split = pieces(&chunk, count, 261_120);
            assert!(split.len() <= count, "{count} handles");
            assert_eq!(
                split.iter().map(|p| u64::from(p.len)).sum::<u64>(),
                u64::from(chunk.len),
                "{count} handles cover the range"
            );
            let mut next = chunk.start;
            for piece in &split {
                assert_eq!(piece.start, next, "no gap and no overlap");
                assert!(u64::from(piece.len) <= u64::from(chunk.len));
                next += u64::from(piece.len);
            }
            assert_eq!(next, chunk.start + u64::from(chunk.len));
        }
    }

    #[test]
    fn a_range_no_longer_than_one_request_stays_whole() {
        let chunk = Chunk {
            index: 0,
            start: 0,
            len: 32_768,
        };
        let split = pieces(&chunk, 8, 261_120);
        assert_eq!(split.len(), 1, "one short request, not eight tiny ones");
        assert_eq!(split[0], chunk);

        let small = Chunk {
            index: 0,
            start: 0,
            len: 4096,
        };
        assert_eq!(
            pieces(&small, 8, 0),
            vec![small],
            "an empty buffer still works"
        );
    }

    #[test]
    fn the_pipeline_depth_is_bounded_by_the_request_count() {
        assert_eq!(pipeline_depth(0), 1);
        assert_eq!(pipeline_depth(1), 1);
        assert_eq!(pipeline_depth(4), 4);
        assert_eq!(pipeline_depth(64), PIPELINE_MAX);
        assert_eq!(pipeline_depth(256), PIPELINE_MAX);
    }

    /// One reader serves every piece: the buffer and its address do not change.
    #[tokio::test]
    async fn the_reader_reuses_one_buffer_for_every_piece() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("source.bin");
        let data: Vec<u8> = (0..8192u32).map(|value| (value % 251) as u8).collect();
        fs::write(&path, &data).unwrap();

        let mut reader = LocalReader::open(&path, 4096).await.unwrap();
        let address = reader.bytes().as_ptr();
        reader.seek_to(0).await.unwrap();
        assert_eq!(reader.fill(4096).await.unwrap(), 4096);
        assert_eq!(reader.bytes(), &data[..4096]);
        reader.seek_to(4096).await.unwrap();
        assert_eq!(reader.fill(4096).await.unwrap(), 4096);
        assert_eq!(reader.bytes(), &data[4096..]);
        assert_eq!(
            reader.bytes().as_ptr(),
            address,
            "the second piece reused the buffer"
        );
    }

    /// One reader per handle, so the pieces of a range read in parallel.
    #[tokio::test]
    async fn the_readers_number_one_per_handle() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("source.bin");
        fs::write(&path, [7u8; 4096]).unwrap();
        let readers = LocalReaders::open(&path, 4096, 4).await.unwrap();
        assert_eq!(readers.len(), 4);
        assert_eq!(LocalReaders::open(&path, 4096, 0).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_short_reader_names_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("short.bin");
        fs::write(&path, [0u8; 16]).unwrap();
        let mut reader = LocalReader::open(&path, 4096).await.unwrap();
        assert_eq!(reader.fill(64).await.expect("the short read"), 16);
        let error = reader.fill(48).await.expect_err("the file ended");
        assert!(error.to_string().contains("short.bin"), "{error}");
    }
}
