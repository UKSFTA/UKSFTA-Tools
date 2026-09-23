use super::legacy::toml_from_legacy;
use super::parse_mod_sources_content;
use crate::error::UksftaError;
use std::fs;
use std::path::Path;

/// Rewrite a legacy mod_sources.txt as TOML v2, keeping a `.bak` backup.
/// Returns true when the file was migrated. A missing or already-TOML file
/// is left untouched. `dry_run` reports without writing.
pub fn migrate_legacy_if_needed(path: &Path, dry_run: bool) -> Result<bool, UksftaError> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };
    let (mods, ignored, is_legacy) = parse_mod_sources_content(&content)?;
    if !is_legacy || content.trim().is_empty() || dry_run {
        return Ok(false);
    }
    let toml = toml_from_legacy(&mods, &ignored)?;
    crate::atomic::write_atomic_with_backup(path, toml.as_bytes())?;
    println!(
        "Migrated mod_sources.txt to TOML format (backup: {})",
        crate::atomic::backup_path(path).display()
    );
    Ok(true)
}
