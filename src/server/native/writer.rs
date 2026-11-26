//! The temp file, its write-only handles, range writes and the run lock.

use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
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
            let stale = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age > Duration::from_secs(6 * 60 * 60));
            if !stale {
                return Err(UksftaError::Input(
                    "another upload is running for this pack".to_string(),
                ));
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

fn create_lock(path: &Path) -> std::io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    writeln!(file, "{} {stamp}", std::process::id())?;
    Ok(())
}

/// Create and truncate the temp file once, then sync it when supported.
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
    if sftp.support_fsync() {
        file.sync_all().await.map_err(sftp_error)?;
    }
    file.close().await.map_err(sftp_error)?;
    Ok(())
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

/// Write one range from its own handle at the explicit offset.
///
/// # Errors
/// Returns `Input` when the local file ends early, the server accepts no
/// bytes, or either side reports an error.
pub async fn write_range(
    file: &mut File,
    chunk: &Chunk,
    local: &Path,
    buffer: u32,
    progress: &mut (dyn FnMut(u64) + Send),
) -> Result<(), UksftaError> {
    file.seek(SeekFrom::Start(chunk.start))
        .await
        .map_err(UksftaError::Io)?;

    let mut source = tokio::fs::File::open(local)
        .await
        .map_err(|error| UksftaError::Input(format!("cannot read {}: {error}", local.display())))?;
    source
        .seek(SeekFrom::Start(chunk.start))
        .await
        .map_err(|error| UksftaError::Input(format!("cannot seek {}: {error}", local.display())))?;

    let mut remaining = u64::from(chunk.len);
    let mut buffer = vec![0u8; usize::try_from(buffer.max(4096)).unwrap_or(4096)];
    while remaining > 0 {
        let want = usize::try_from(remaining.min(buffer.len() as u64)).unwrap_or(buffer.len());
        let read = source.read(&mut buffer[..want]).await.map_err(|error| {
            UksftaError::Input(format!("cannot read {}: {error}", local.display()))
        })?;
        if read == 0 {
            return Err(UksftaError::Input(format!(
                "the local file ended early: {}",
                local.display()
            )));
        }
        let mut written = 0;
        while written < read {
            let count = file
                .write(&buffer[written..read])
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
        progress(read as u64);
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

/// A shared queue of ranges for the workers.
pub fn queue(chunks: &[Chunk]) -> Arc<Mutex<VecDeque<Chunk>>> {
    Arc::new(Mutex::new(chunks.iter().copied().collect()))
}

fn sftp_error(error: openssh_sftp_client::Error) -> UksftaError {
    UksftaError::Input(format!("sftp: {error}"))
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
    fn the_queue_holds_every_range_once() {
        let chunks = vec![
            Chunk {
                index: 0,
                start: 0,
                len: 4096,
            },
            Chunk {
                index: 1,
                start: 4096,
                len: 4096,
            },
        ];
        let queue = queue(&chunks);
        assert_eq!(queue.lock().unwrap().len(), 2);
    }
}
