use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::UksftaError;

// --- Lock file ---

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct LockFile {
    #[serde(default = "default_lock_version")]
    pub version: u32,
    pub mods: HashMap<String, ModLockEntry>,
}

pub fn default_lock_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ModLockEntry {
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
    #[serde(default)]
    pub updated: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Dependency {
    pub id: String,
    pub name: String,
}

/// Load the lock file. A missing file is an empty lock. A present but
/// unparseable file is a hard error, so a corrupt lock is never silently
/// replaced by an empty one.
pub fn load_lock(path: &Path) -> Result<LockFile, UksftaError> {
    if path.exists() {
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content)
            .map_err(|e| UksftaError::Parse(format!("failed to parse {}: {}", path.display(), e)))
    } else {
        Ok(LockFile {
            version: default_lock_version(),
            mods: HashMap::new(),
        })
    }
}

/// Write the lock file atomically, keeping the previous lock as `{path}.bak`.
pub fn save_lock(path: &Path, lock: &LockFile) -> Result<(), UksftaError> {
    let json = serde_json::to_string_pretty(lock)
        .map_err(|e| UksftaError::Parse(format!("failed to serialize lock: {e}")))?;
    crate::atomic::write_atomic_with_backup(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn load_lock_errors_on_corrupt_file() {
        let path = temp_path("uksfta-lock-corrupt.json");
        fs::write(&path, "{").unwrap();
        assert!(load_lock(&path).is_err());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_lock_missing_file_is_empty() {
        let path = temp_path("uksfta-lock-absent.json");
        let _ = fs::remove_file(&path);
        assert!(load_lock(&path).unwrap().mods.is_empty());
    }

    #[test]
    fn mod_lock_entry_without_optional_fields_loads() {
        let json = r#"{"version":1,"mods":{"111":{"name":"Mod"}}}"#;
        let lock: LockFile = serde_json::from_str(json).unwrap();
        let entry = &lock.mods["111"];
        assert!(entry.files.is_empty());
        assert!(entry.tags.is_empty());
        assert!(entry.dependencies.is_empty());
        assert_eq!(entry.updated, "");
    }

    #[test]
    fn save_lock_keeps_previous_lock_as_backup() {
        let path = temp_path("uksfta-lock-save.json");
        let backup = crate::atomic::backup_path(&path);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);

        let first = LockFile {
            version: 1,
            mods: HashMap::new(),
        };
        save_lock(&path, &first).unwrap();
        // The first save has no previous file, so there is nothing to back up.
        assert!(!backup.exists());

        let mut second = LockFile {
            version: 1,
            mods: HashMap::new(),
        };
        second.mods.insert(
            "111".to_string(),
            ModLockEntry {
                files: vec!["addons/x.pbo".to_string()],
                name: "Mod".to_string(),
                tags: Vec::new(),
                dependencies: Vec::new(),
                updated: "0".to_string(),
            },
        );
        save_lock(&path, &second).unwrap();

        assert!(backup.exists());
        let restored: LockFile =
            serde_json::from_str(&fs::read_to_string(&backup).unwrap()).unwrap();
        assert_eq!(restored, first);
        assert_eq!(load_lock(&path).unwrap(), second);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);
    }
}
