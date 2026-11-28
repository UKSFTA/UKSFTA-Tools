//! Hash a remote file without writing it to disk.

use std::fmt::Write;
use std::path::Path;

use bytes::BytesMut;
use openssh_sftp_client::file::File;
use openssh_sftp_client::Sftp;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncSeekExt, SeekFrom};

use super::connect;
use super::fanout;
use crate::error::UksftaError;
use crate::server::config::Settings;

/// The buffer one read-back request uses. It is deliberately not the
/// transfer buffer: the read-back is one stream and benefits from a large
/// request.
pub const READBACK_BUFFER: u32 = 261_120;

/// The most read-back requests one connection keeps in flight. Eight large
/// requests hold about 2 MiB, which fits the smallest request window.
pub const READBACK_PIPELINE: usize = 8;

/// Hash the whole remote file and fail when its length differs.
///
/// # Errors
/// Returns `Input` for a read error or a length mismatch.
pub async fn hash_remote(
    sftp: &Sftp,
    path: &str,
    expected_len: u64,
    buffer: u32,
) -> Result<String, UksftaError> {
    hash(sftp, path, expected_len, buffer).await
}

/// Hash the first `len` bytes of the remote file. A resume compares it with
/// the local prefix before it continues.
///
/// # Errors
/// Returns `Input` for a read error or a file shorter than `len`.
pub async fn hash_remote_prefix(
    sftp: &Sftp,
    path: &str,
    len: u64,
    buffer: u32,
) -> Result<String, UksftaError> {
    hash(sftp, path, len, buffer).await
}

/// The digest of each remote path in the order it was asked for. `None` is a
/// file that could not be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamedHashes {
    pub digests: Vec<Option<String>>,
}

/// Hash the remote paths in order over one native session, with no local
/// copy on disk. Each path carries the length the listing reported, and the
/// same pipelined reader the completion hash uses does the work.
///
/// # Errors
/// Returns the connection error when the session cannot be opened, so the
/// caller can fall back. A path that cannot be read is `None` in the result.
pub fn stream_hashes(
    settings: &Settings,
    paths: &[(String, u64)],
) -> Result<StreamedHashes, UksftaError> {
    if paths.is_empty() {
        return Ok(StreamedHashes::default());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(UksftaError::Io)?;
    runtime.block_on(async {
        let session = connect::connect(settings).await?;
        let mut digests = Vec::with_capacity(paths.len());
        for (path, len) in paths {
            digests.push(
                hash_remote(&session.sftp, path, *len, READBACK_BUFFER)
                    .await
                    .ok(),
            );
        }
        Ok(StreamedHashes { digests })
    })
}

async fn hash(sftp: &Sftp, path: &str, len: u64, buffer: u32) -> Result<String, UksftaError> {
    let file = sftp
        .open(path)
        .await
        .map_err(|error| UksftaError::Input(format!("cannot read {path}: {error}")))?;
    let readers = Reader::remote_set(file, READBACK_PIPELINE, buffer);
    let hasher = read_into_hasher(readers, path, len, buffer).await?;

    Ok(hex_digest(&hasher.finalize()))
}

/// Read every range of `len` bytes and hash them as one stream in file order.
///
/// One read per reader is outstanding at a time, so at most `readers.len()`
/// ranges are in flight and at most that many wait for their turn. The digest
/// is a single SHA-256, so the ranges are absorbed in offset order whatever
/// order the reads finish in.
async fn read_into_hasher(
    mut readers: Vec<Reader>,
    path: &str,
    len: u64,
    buffer: u32,
) -> Result<Sha256, UksftaError> {
    let want_max = buffer.max(4096);
    let mut hasher = Sha256::new();
    let mut offset = 0u64;

    while offset < len {
        let mut windows: Vec<(u64, u32)> = Vec::with_capacity(readers.len());
        while windows.len() < readers.len() && offset < len {
            let want = u32::try_from((len - offset).min(u64::from(want_max))).unwrap_or(want_max);
            windows.push((offset, want));
            offset += u64::from(want);
        }

        let futures = readers
            .iter_mut()
            .zip(&windows)
            .map(|(reader, &(start, want))| reader.read_at(path, start, want, len))
            .collect::<Vec<_>>();
        let results = fanout::all(futures).await;

        let mut rows: Vec<(u64, usize, BytesMut)> = Vec::with_capacity(windows.len());
        for (index, (window, result)) in windows.iter().zip(results).enumerate() {
            let data = result?;
            // A short read means the file is shorter than the caller expects.
            // The offset is advanced by the request, not by the bytes that
            // arrived, so this check is what catches a short file.
            if u64::try_from(data.len()).unwrap_or(u64::MAX) != u64::from(window.1) {
                return Err(short(path, window.0 + data.len() as u64, len));
            }
            rows.push((window.0, index, data));
        }
        rows.sort_unstable_by_key(|(start, _, _)| *start);
        for (_, index, data) in rows {
            hasher.update(&data);
            readers[index].recycle(data);
        }
    }

    Ok(hasher)
}

/// One read-back handle and the buffer it reuses.
struct Reader {
    source: Source,
    spare: BytesMut,
}

enum Source {
    Remote(File),
    #[cfg(test)]
    Local(std::fs::File),
}

impl Reader {
    /// One reader per handle. The handles share the server's file handle, so
    /// each keeps its own offset.
    fn remote_set(base: File, count: usize, capacity: u32) -> Vec<Self> {
        (0..count.max(1))
            .map(|_| Self {
                source: Source::Remote(base.clone()),
                spare: BytesMut::with_capacity(usize::try_from(capacity.max(4096)).unwrap_or(4096)),
            })
            .collect()
    }

    /// Read `want` bytes at `start`. A short read means the file is shorter
    /// than the caller expects.
    async fn read_at(
        &mut self,
        path: &str,
        start: u64,
        want: u32,
        total: u64,
    ) -> Result<BytesMut, UksftaError> {
        let mut buffer = std::mem::take(&mut self.spare);
        if buffer.capacity() < usize::try_from(want).unwrap_or(0) {
            buffer = BytesMut::with_capacity(usize::try_from(want).unwrap_or(4096));
        }

        match &mut self.source {
            Source::Remote(file) => {
                // The crate advances its own offset by the requested length,
                // not by the bytes returned, so the offset is set explicitly.
                file.seek(SeekFrom::Start(start))
                    .await
                    .map_err(|error| UksftaError::Input(format!("cannot read {path}: {error}")))?;
                let Some(data) = file
                    .read(want, buffer)
                    .await
                    .map_err(|error| UksftaError::Input(format!("cannot read {path}: {error}")))?
                else {
                    return Err(short(path, start, total));
                };
                if data.is_empty() {
                    return Err(short(path, start, total));
                }
                Ok(data)
            }
            #[cfg(test)]
            Source::Local(file) => {
                use std::io::{Read, Seek, SeekFrom as LocalStart};

                file.seek(LocalStart::Start(start))
                    .map_err(|error| UksftaError::Input(format!("cannot read {path}: {error}")))?;
                let mut buffer = buffer;
                buffer.resize(usize::try_from(want).unwrap_or(0), 0);
                let mut filled = 0;
                while filled < buffer.len() {
                    let read = file.read(&mut buffer[filled..]).map_err(|error| {
                        UksftaError::Input(format!("cannot read {path}: {error}"))
                    })?;
                    if read == 0 {
                        return Err(short(path, start, total));
                    }
                    filled += read;
                }
                Ok(buffer)
            }
        }
    }

    /// Keep the allocation for the next read.
    fn recycle(&mut self, mut data: BytesMut) {
        data.clear();
        if data.capacity() >= self.spare.capacity() {
            self.spare = data;
        }
    }
}

fn short(path: &str, held: u64, total: u64) -> UksftaError {
    UksftaError::Input(format!(
        "the server holds {held} bytes of {path}, expected {total}"
    ))
}

/// Hash the first `len` bytes of the local file.
///
/// # Errors
/// Returns `Input` when the file cannot be read or is shorter than `len`.
pub fn hash_local_prefix(path: &Path, len: u64) -> Result<String, UksftaError> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .map_err(|error| UksftaError::Input(format!("cannot read {}: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut left = len;
    while left > 0 {
        let want = usize::try_from(left.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = file.read(&mut buffer[..want]).map_err(|error| {
            UksftaError::Input(format!("cannot read {}: {error}", path.display()))
        })?;
        if read == 0 {
            return Err(UksftaError::Input(format!(
                "the local file ended early: {}",
                path.display()
            )));
        }
        hasher.update(&buffer[..read]);
        left -= read as u64;
    }
    Ok(hex_digest(&hasher.finalize()))
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The range size these tests use: small enough to need many windows and
    /// not a divisor of the file length.
    const WANT: u32 = 64 * 1024;

    fn local_readers(path: &Path, count: usize) -> Vec<Reader> {
        (0..count)
            .map(|_| Reader {
                source: Source::Local(std::fs::File::open(path).expect("open the local file")),
                spare: BytesMut::with_capacity(usize::try_from(WANT).unwrap_or(4096)),
            })
            .collect()
    }

    fn sequential(path: &Path) -> String {
        let bytes = std::fs::read(path).expect("read the local file");
        hex_digest(&Sha256::digest(&bytes))
    }

    fn payload(size: usize) -> Vec<u8> {
        (0..size).map(|index| (index % 251) as u8).collect()
    }

    #[tokio::test]
    async fn the_pipelined_digest_matches_the_sequential_one() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("ranges.bin");
        // Larger than one range and not a whole number of ranges.
        let size = 3 * 1024 * 1024 + 517;
        std::fs::write(&path, payload(size)).expect("write the file");
        let expected = sequential(&path);

        for readers in [1usize, 3, 4, 8] {
            let set = local_readers(&path, readers);
            let hasher = read_into_hasher(set, "", size as u64, WANT)
                .await
                .expect("the pipelined digest");
            assert_eq!(
                hex_digest(&hasher.finalize()),
                expected,
                "{readers} concurrent readers"
            );
        }
    }

    #[tokio::test]
    async fn the_digest_holds_when_the_length_is_not_a_whole_number_of_ranges() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("short-last.bin");
        let size = 5 * usize::try_from(WANT).unwrap_or(4096) + 1;
        std::fs::write(&path, payload(size)).expect("write the file");

        let set = local_readers(&path, 4);
        let hasher = read_into_hasher(set, "", size as u64, WANT)
            .await
            .expect("the pipelined digest");
        assert_eq!(hex_digest(&hasher.finalize()), sequential(&path));
    }

    #[tokio::test]
    async fn a_file_shorter_than_the_expected_length_is_named() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("short.bin");
        let size = usize::try_from(WANT).unwrap_or(4096) + 100;
        std::fs::write(&path, payload(size)).expect("write the file");

        let set = local_readers(&path, 4);
        let error = read_into_hasher(set, "", size as u64 + 4096, WANT)
            .await
            .expect_err("the file is short");
        let text = error.to_string();
        assert!(text.contains("expected"), "{text}");
    }
}
