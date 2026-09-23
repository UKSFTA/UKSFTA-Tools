//! Atomic file writes. A rename replaces the destination on both Unix and
//! Windows, so a reader never sees a half-written file.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::UksftaError;

/// Write `bytes` to `path` through a sibling temp file and a rename.
/// The temp file is removed when the write or the rename fails.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), UksftaError> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("uksfta.tmp");
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".{}.{}.tmp", file_name, std::process::id()));

    if let Err(e) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(UksftaError::Io(e));
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(UksftaError::Io(e));
    }
    Ok(())
}

/// Copy an existing `path` to `{path}.bak`, then call [`write_atomic`].
/// A path that does not exist is written without a backup.
pub fn write_atomic_with_backup(path: &Path, bytes: &[u8]) -> Result<(), UksftaError> {
    if path.exists() {
        fs::copy(path, backup_path(path)).map_err(UksftaError::Io)?;
    }
    write_atomic(path, bytes)
}

/// The backup path for `path`: the full file name plus `.bak`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".bak");
    PathBuf::from(os)
}
