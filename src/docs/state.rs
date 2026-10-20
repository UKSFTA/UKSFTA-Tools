//! The saved composition baseline. It must be committed so a fresh clone
//! and CI can tell which mods changed since the last release.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::atomic::write_atomic_with_backup;
use crate::error::UksftaError;

/// One installed Workshop item as it appeared in `mods.lock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModSnapshot {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub updated: String,
}

pub type ModMap = BTreeMap<String, ModSnapshot>;

/// The baseline for the current version, and the composition at the last
/// run so a version bump can start a new baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishState {
    #[serde(default = "schema_version")]
    pub schema: u32,
    pub project_version: String,
    #[serde(default)]
    pub baseline: ModMap,
    #[serde(default)]
    pub last: ModMap,
    /// Set while the first tracked version is current, so every run can
    /// reproduce its initial-release entry.
    #[serde(default)]
    pub initial: Option<usize>,
}

fn schema_version() -> u32 {
    1
}

/// Read the state file. A missing file is `None`; an unparseable file is an
/// error, so a corrupt baseline is never replaced with an empty one.
pub fn load(path: &Path) -> Result<Option<PublishState>, UksftaError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    let state = serde_json::from_str(&text)
        .map_err(|e| UksftaError::Parse(format!("failed to parse {}: {e}", path.display())))?;
    Ok(Some(state))
}

/// Write the state atomically. The parent directory is created on first use.
pub fn save(path: &Path, state: &PublishState) -> Result<(), UksftaError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| UksftaError::Parse(format!("failed to serialize state: {e}")))?;
    write_atomic_with_backup(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("uksfta-docs-state-{name}-{}", std::process::id()))
    }

    fn sample() -> PublishState {
        let mut baseline = ModMap::new();
        baseline.insert(
            "1".into(),
            ModSnapshot {
                name: "Alpha".into(),
                updated: "100".into(),
            },
        );
        PublishState {
            schema: 1,
            project_version: "1.0.0.0".into(),
            baseline: baseline.clone(),
            last: baseline,
            initial: Some(1),
        }
    }

    #[test]
    fn saves_and_loads() {
        let path = temp_path("roundtrip");
        let _ = fs::remove_file(&path);
        save(&path, &sample()).unwrap();
        assert_eq!(load(&path).unwrap(), Some(sample()));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(crate::atomic::backup_path(&path));
    }

    #[test]
    fn missing_file_is_none() {
        let path = temp_path("missing");
        let _ = fs::remove_file(&path);
        assert_eq!(load(&path).unwrap(), None);
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let path = temp_path("corrupt");
        fs::write(&path, "{").unwrap();
        assert!(load(&path).is_err());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn state_without_optional_fields_loads() {
        let state: PublishState =
            serde_json::from_str(r#"{"project_version":"1.0.0.0","baseline":{"1":{"name":"A"}}}"#)
                .unwrap();
        assert_eq!(state.schema, 1);
        assert_eq!(state.baseline["1"].updated, "");
        assert!(state.last.is_empty());
        assert_eq!(state.initial, None);
    }
}
