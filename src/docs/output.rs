//! File writes, overwrite guards, and status helpers.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use super::changelog::Changes;
use super::DocsOptions;
use crate::atomic::write_atomic_with_backup;
use crate::error::UksftaError;

/// Report a target that cannot be updated. `--check` treats it as stale; a
/// normal run leaves the file alone unless the caller can retry.
pub(super) fn report_blocked(
    path: &Path,
    options: &DocsOptions<'_>,
    stale: &mut Vec<String>,
    reason: &str,
) {
    if options.check {
        println!("{}: {reason}", path.display());
        stale.push(path.display().to_string());
    } else {
        eprintln!(
            "Warning: {}: {reason}. Use --force to replace it.",
            path.display()
        );
    }
}

/// A short composition summary for the status line.
pub(super) fn summarize(changes: &Changes) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(count) = changes.initial {
        let noun = if count == 1 { "mod" } else { "mods" };
        parts.push(format!("initial release with {count} {noun}"));
    }
    let set = &changes.set;
    if !set.added.is_empty() {
        parts.push(format!("+{} added", set.added.len()));
    }
    if !set.updated.is_empty() {
        parts.push(format!("{} updated", set.updated.len()));
    }
    if !set.removed.is_empty() {
        parts.push(format!("-{} removed", set.removed.len()));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

pub(super) fn write_target(
    path: &Path,
    content: &str,
    options: &DocsOptions<'_>,
    stale: &mut Vec<String>,
) -> Result<bool, UksftaError> {
    let existing = read_existing(path)?;
    let changed = existing.as_deref() != Some(content);

    if options.check {
        if changed {
            println!("{}: out of date", path.display());
            stale.push(path.display().to_string());
        }
        return Ok(changed);
    }
    if options.dry_run {
        println!("--- {} ---", path.display());
        print!("{content}");
        return Ok(changed);
    }
    if changed {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        write_atomic_with_backup(path, content.as_bytes())?;
    }
    Ok(changed)
}

/// Read a target file. A missing file is `None`; any other read failure is
/// an error, so a permission problem is not mistaken for a new file.
pub(super) fn read_existing(path: &Path) -> Result<Option<String>, UksftaError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(UksftaError::Io(e)),
    }
}

/// The HEMTT name, or the project directory name when the key is absent.
pub(super) fn project_name(name: &Option<String>, root: &Path) -> String {
    if let Some(name) = name {
        return name.clone();
    }
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string())
}
