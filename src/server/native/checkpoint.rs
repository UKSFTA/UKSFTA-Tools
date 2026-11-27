//! The resume checkpoint. It records only the ranges the server acknowledged,
//! together with the identity of the local file and the temp file name.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use openssh_sftp_client::Sftp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::verify;
use super::writer::plan_temp_name;
use crate::error::UksftaError;
use crate::server::config::{SegmentResume, Settings};
use crate::server::report::UploadUnit;

/// The only checkpoint format this build reads.
pub const VERSION: u8 = 1;

/// What one interrupted run leaves behind.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct Checkpoint {
    pub version: u8,
    pub remote: String,
    pub temp: String,
    pub local_sha256: String,
    pub local_size: u64,
    pub local_mtime: i64,
    pub chunk: u32,
    pub acked_prefix: u64,
    pub updated_unix: i64,
}

/// The local facts a checkpoint is valid against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalIdentity {
    pub size: u64,
    pub mtime: i64,
    pub sha256: String,
}

/// The local file's identity. The size and time are read at once, and the
/// content hash runs on a blocking thread until someone needs it, so a run
/// does not wait for the hash before its first range.
pub struct IdentityJob {
    size: u64,
    mtime: i64,
    result: tokio::sync::OnceCell<LocalIdentity>,
    hash: tokio::sync::Mutex<Option<tokio::task::JoinHandle<Result<String, UksftaError>>>>,
}

impl IdentityJob {
    /// Read the size and time, and start the hash beside the caller.
    ///
    /// # Errors
    /// Returns `Input` when the file cannot be read.
    pub fn start(path: &Path) -> Result<Self, UksftaError> {
        let (size, mtime) = metadata_of(path)?;
        let owned = path.to_path_buf();
        let hash = tokio::task::spawn_blocking(move || {
            crate::util::file_sha256_with_buffer(&owned, crate::util::HASH_BUFFER_LARGE)
                .ok_or_else(|| UksftaError::Input(format!("cannot hash {}", owned.display())))
        });
        Ok(Self {
            size,
            mtime,
            result: tokio::sync::OnceCell::new(),
            hash: tokio::sync::Mutex::new(Some(hash)),
        })
    }

    /// The size read at the start.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The modification time read at the start.
    pub fn mtime(&self) -> i64 {
        self.mtime
    }

    /// Join the hash. The first caller waits for the blocking task, later
    /// callers reuse the result.
    ///
    /// # Errors
    /// Returns `Input` when the file cannot be hashed.
    pub async fn finish(&self) -> Result<LocalIdentity, UksftaError> {
        self.result
            .get_or_try_init(|| async {
                let handle = self.hash.lock().await.take().ok_or_else(|| {
                    UksftaError::Input("the local hash was already taken".to_string())
                })?;
                let sha256 = handle.await.map_err(|error| {
                    UksftaError::Input(format!("the local hash task stopped: {error}"))
                })??;
                Ok(LocalIdentity {
                    size: self.size,
                    mtime: self.mtime,
                    sha256,
                })
            })
            .await
            .cloned()
    }
}

/// The size and modification time of the local file.
fn metadata_of(path: &Path) -> Result<(u64, i64), UksftaError> {
    let metadata = fs::metadata(path)
        .map_err(|error| UksftaError::Input(format!("cannot read {}: {error}", path.display())))?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default();
    Ok((metadata.len(), mtime))
}

/// Where the checkpoint for one remote file lives.
pub fn path_for(state_root: &Path, pack_id: &str, remote: &str) -> PathBuf {
    let digest = hex(&Sha256::digest(remote.as_bytes()));
    let short: String = digest.chars().take(16).collect();
    state_root
        .join(".uksfta")
        .join("upload")
        .join(pack_id)
        .join(format!("{short}.json"))
}

/// A checkpoint with nothing acknowledged.
pub fn fresh(remote: &str, temp: &str, chunk: u32, local: &LocalIdentity) -> Checkpoint {
    Checkpoint {
        version: VERSION,
        remote: remote.to_string(),
        temp: temp.to_string(),
        local_sha256: local.sha256.clone(),
        local_size: local.size,
        local_mtime: local.mtime,
        chunk,
        acked_prefix: 0,
        updated_unix: now(),
    }
}

/// Read a checkpoint. A missing or unreadable file is `None`.
pub fn load(path: &Path) -> Option<Checkpoint> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Write the checkpoint atomically: a temp file, fsync, rename, fsync the
/// directory.
///
/// # Errors
/// Returns `Input` when the directory or the file cannot be written.
pub fn save(path: &Path, checkpoint: &Checkpoint) -> Result<(), UksftaError> {
    let parent = path
        .parent()
        .ok_or_else(|| UksftaError::Input(format!("no directory for {}", path.display())))?;
    fs::create_dir_all(parent).map_err(UksftaError::Io)?;
    let temp = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "checkpoint".to_string()),
        std::process::id()
    ));
    let text = serde_json::to_string(checkpoint)
        .map_err(|error| UksftaError::Input(format!("cannot encode the checkpoint: {error}")))?;
    {
        let mut file = fs::File::create(&temp).map_err(UksftaError::Io)?;
        file.write_all(text.as_bytes()).map_err(UksftaError::Io)?;
        file.sync_all().map_err(UksftaError::Io)?;
    }
    fs::rename(&temp, path).map_err(UksftaError::Io)?;
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

/// The largest prefix of complete ranges. The short last range caps the
/// prefix at the file size.
pub fn recompute_prefix(checkpoint: &mut Checkpoint, done: &BTreeSet<u32>, size: u64) {
    let mut count = 0u64;
    while done.contains(&u32::try_from(count).unwrap_or(u32::MAX)) {
        count += 1;
    }
    checkpoint.acked_prefix = (count.saturating_mul(u64::from(checkpoint.chunk))).min(size);
}

/// Where a resumed run starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// Send every range again.
    Fresh,
    /// Everything below `from` is acknowledged and verified.
    Resume { from: u64 },
}

/// Why a checkpoint cannot be trusted, judged from the file's size and time
/// only. A run that fails here never hashes the local file.
fn staged_mismatch(
    checkpoint: &Checkpoint,
    unit: &UploadUnit,
    size: u64,
    mtime: i64,
) -> Option<&'static str> {
    if checkpoint.version != VERSION {
        return Some("the checkpoint version changed");
    }
    if checkpoint.remote != unit.remote {
        return Some("the checkpoint names another file");
    }
    if checkpoint.temp != plan_temp_name(&unit.remote) {
        return Some("the checkpoint names another temp file");
    }
    if checkpoint.local_size != size {
        return Some("the local size changed");
    }
    if checkpoint.local_mtime != mtime {
        return Some("the local modification time changed");
    }
    None
}

/// Why a checkpoint cannot be trusted.
fn mismatch(
    checkpoint: &Checkpoint,
    unit: &UploadUnit,
    identity: &LocalIdentity,
) -> Option<&'static str> {
    if checkpoint.version != VERSION {
        return Some("the checkpoint version changed");
    }
    if checkpoint.remote != unit.remote {
        return Some("the checkpoint names another file");
    }
    if checkpoint.temp != plan_temp_name(&unit.remote) {
        return Some("the checkpoint names another temp file");
    }
    if checkpoint.local_size != identity.size {
        return Some("the local size changed");
    }
    if checkpoint.local_mtime != identity.mtime {
        return Some("the local modification time changed");
    }
    if checkpoint.local_sha256 != identity.sha256 {
        return Some("the local content changed");
    }
    if checkpoint.acked_prefix > unit.size {
        return Some("the acknowledged prefix is past the end of the file");
    }
    if checkpoint.chunk == 0 {
        return Some("the checkpoint has no chunk size");
    }
    let chunk = u64::from(checkpoint.chunk);
    if checkpoint.acked_prefix != unit.size && !checkpoint.acked_prefix.is_multiple_of(chunk) {
        return Some("the acknowledged prefix is not a chunk boundary");
    }
    None
}

/// Choose the start point. The remote size is never trusted: the acknowledged
/// prefix is hashed against the local prefix before it is continued.
///
/// # Errors
/// Returns `Input` when the local file cannot be read or the prefix hashed.
pub async fn decide(
    sftp: &Sftp,
    state_root: &Path,
    pack_id: &str,
    unit: &UploadUnit,
    settings: &Settings,
    identity: &IdentityJob,
) -> Result<(Start, String), UksftaError> {
    let temp = plan_temp_name(&unit.remote);
    let path = path_for(state_root, pack_id, &unit.remote);
    let name = remote_name(&unit.remote);

    if settings.segment.resume == SegmentResume::Off {
        discard(sftp, &path, &temp).await;
        return Ok((Start::Fresh, temp));
    }

    let Some(mut checkpoint) = load(&path) else {
        // No checkpoint: a temp file can only be an interrupted transfer that
        // was never acknowledged.
        let _ = sftp.fs().remove_file(&temp).await;
        return Ok((Start::Fresh, temp));
    };

    let staged = staged_mismatch(&checkpoint, unit, identity.size(), identity.mtime());
    let identity = match staged {
        Some(reason) => {
            eprintln!("Warning: restarting {name}: {reason}");
            discard(sftp, &path, &temp).await;
            return Ok((Start::Fresh, temp));
        }
        None => identity.finish().await?,
    };
    if let Some(reason) = mismatch(&checkpoint, unit, &identity) {
        eprintln!("Warning: restarting {name}: {reason}");
        discard(sftp, &path, &temp).await;
        return Ok((Start::Fresh, temp));
    }

    if checkpoint.acked_prefix == 0 {
        // Nothing was acknowledged, so there is nothing to trust or lose.
        discard(sftp, &path, &temp).await;
        return Ok((Start::Fresh, temp));
    }

    if sftp.fs().metadata(&temp).await.is_err() {
        eprintln!("Warning: restarting {name}: the temp file is gone");
        discard(sftp, &path, &temp).await;
        return Ok((Start::Fresh, temp));
    }

    if checkpoint.acked_prefix == unit.size {
        match verify::hash_remote(sftp, &temp, unit.size, verify::READBACK_BUFFER).await {
            Ok(hash) if hash == identity.sha256 => {
                return Ok((Start::Resume { from: unit.size }, temp));
            }
            _ => {
                eprintln!(
                    "Warning: restarting {name}: the temp file does not match the local file"
                );
                discard(sftp, &path, &temp).await;
                return Ok((Start::Fresh, temp));
            }
        }
    }

    let local_prefix = verify::hash_local_prefix(&unit.local, checkpoint.acked_prefix)?;
    match verify::hash_remote_prefix(
        sftp,
        &temp,
        checkpoint.acked_prefix,
        verify::READBACK_BUFFER,
    )
    .await
    {
        Ok(hash) if hash == local_prefix => {
            checkpoint.updated_unix = now();
            Ok((
                Start::Resume {
                    from: checkpoint.acked_prefix,
                },
                temp,
            ))
        }
        _ => {
            eprintln!("Warning: restarting {name}: the temp file holds different bytes");
            discard(sftp, &path, &temp).await;
            Ok((Start::Fresh, temp))
        }
    }
}

/// Remove the temp file and the checkpoint. Both are best effort.
async fn discard(sftp: &Sftp, path: &Path, temp: &str) {
    let _ = fs::remove_file(path);
    let _ = sftp.fs().remove_file(temp).await;
}

/// The last path segment of a remote path.
pub fn remote_name(remote: &str) -> &str {
    remote.rsplit('/').next().unwrap_or(remote)
}

/// Seconds since the epoch.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(size: u64) -> LocalIdentity {
        LocalIdentity {
            size,
            mtime: 1_700_000_000,
            sha256: "a".repeat(64),
        }
    }

    fn unit(size: u64) -> UploadUnit {
        UploadUnit {
            local: PathBuf::from("/local/big.pbo"),
            remote: "/srv/arma/@1/addons/big.pbo".to_string(),
            size,
        }
    }

    fn checkpoint(size: u64, chunk: u32, prefix: u64) -> Checkpoint {
        let mut checkpoint = fresh(
            &unit(size).remote,
            &plan_temp_name(&unit(size).remote),
            chunk,
            &identity(size),
        );
        checkpoint.acked_prefix = prefix;
        checkpoint
    }

    #[test]
    fn a_checkpoint_round_trips_through_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("cp.json");
        let original = checkpoint(8 * 1024 * 1024, 1024 * 1024, 2 * 1024 * 1024);
        save(&path, &original).unwrap();
        assert_eq!(load(&path), Some(original));
    }

    #[test]
    fn save_leaves_no_temporary_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("cp.json");
        save(&path, &checkpoint(1024, 1024, 0)).unwrap();
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["cp.json".to_string()], "{names:?}");
    }

    #[test]
    fn the_prefix_stops_at_the_first_gap() {
        let mut checkpoint = checkpoint(4 * 1024 * 1024, 1024 * 1024, 0);
        let done: BTreeSet<u32> = [0, 1, 3].into_iter().collect();
        recompute_prefix(&mut checkpoint, &done, 4 * 1024 * 1024);
        assert_eq!(checkpoint.acked_prefix, 2 * 1024 * 1024, "index 2 is a gap");
    }

    #[test]
    fn the_short_last_range_caps_the_prefix() {
        let size = 3 * 1024 * 1024 + 4096;
        let mut checkpoint = checkpoint(size, 1024 * 1024, 0);
        let done: BTreeSet<u32> = [0, 1, 2, 3].into_iter().collect();
        recompute_prefix(&mut checkpoint, &done, size);
        assert_eq!(checkpoint.acked_prefix, size);
    }

    #[test]
    fn an_empty_checkpoint_has_no_prefix() {
        let mut checkpoint = checkpoint(1024 * 1024, 1024 * 1024, 0);
        recompute_prefix(&mut checkpoint, &BTreeSet::new(), 1024 * 1024);
        assert_eq!(checkpoint.acked_prefix, 0);
    }

    #[test]
    fn each_rejection_reason_is_named() {
        let size = 4 * 1024 * 1024;
        let good = checkpoint(size, 1024 * 1024, 1024 * 1024);
        assert!(mismatch(&good, &unit(size), &identity(size)).is_none());

        let mut wrong_version = good.clone();
        wrong_version.version = 2;
        assert_eq!(
            mismatch(&wrong_version, &unit(size), &identity(size)),
            Some("the checkpoint version changed")
        );

        let mut wrong_remote = good.clone();
        wrong_remote.remote = "/srv/arma/@1/addons/other.pbo".to_string();
        assert!(mismatch(&wrong_remote, &unit(size), &identity(size)).is_some());

        let mut wrong_temp = good.clone();
        wrong_temp.temp = "/srv/arma/@1/addons/big.pbo.part".to_string();
        assert!(mismatch(&wrong_temp, &unit(size), &identity(size)).is_some());

        let mut wrong_size = good.clone();
        wrong_size.local_size += 1;
        assert_eq!(
            mismatch(&wrong_size, &unit(size), &identity(size)),
            Some("the local size changed")
        );

        let mut wrong_mtime = good.clone();
        wrong_mtime.local_mtime -= 1;
        assert_eq!(
            mismatch(&wrong_mtime, &unit(size), &identity(size)),
            Some("the local modification time changed")
        );

        let mut wrong_hash = good.clone();
        wrong_hash.local_sha256 = "b".repeat(64);
        assert_eq!(
            mismatch(&wrong_hash, &unit(size), &identity(size)),
            Some("the local content changed")
        );

        let mut past_end = good.clone();
        past_end.acked_prefix = size + 1024 * 1024;
        assert!(mismatch(&past_end, &unit(size), &identity(size)).is_some());

        let mut unaligned = good;
        unaligned.acked_prefix = 1024 * 1024 + 4096;
        assert_eq!(
            mismatch(&unaligned, &unit(size), &identity(size)),
            Some("the acknowledged prefix is not a chunk boundary")
        );
    }

    #[test]
    fn the_checkpoint_path_is_sixteen_hex_characters() {
        let path = path_for(
            Path::new("/repo"),
            "3662499431",
            "/srv/arma/@1/addons/big.pbo",
        );
        assert!(path.starts_with("/repo/.uksfta/upload/3662499431/"));
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(name.len(), 16, "{name}");
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()), "{name}");
    }

    #[test]
    fn a_missing_checkpoint_is_none() {
        assert!(load(Path::new("/nonexistent/checkpoint.json")).is_none());
    }

    #[test]
    fn the_remote_name_is_the_last_segment() {
        assert_eq!(remote_name("/srv/a/b.pbo"), "b.pbo");
        assert_eq!(remote_name("b.pbo"), "b.pbo");
    }
}
