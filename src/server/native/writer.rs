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
    let stat = fs.statvfs(dir).await.map_err(|error| {
        UksftaError::Input(format!("cannot ask for free space at {dir}: {error}"))
    })?;
    let free = stat.bavail.saturating_mul(stat.frsize);
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
}
