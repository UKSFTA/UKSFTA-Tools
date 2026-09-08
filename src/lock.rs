use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

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
    pub files: Vec<String>,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub dependencies: Vec<Dependency>,
    pub updated: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Dependency {
    pub id: String,
    pub name: String,
}

pub fn load_lock(path: &Path) -> LockFile {
    if path.exists() {
        let content = fs::read_to_string(path).expect("Failed to read mods.lock");
        serde_json::from_str(&content).unwrap_or(LockFile {
            version: default_lock_version(),
            mods: HashMap::new(),
        })
    } else {
        LockFile {
            version: default_lock_version(),
            mods: HashMap::new(),
        }
    }
}

pub fn save_lock(path: &Path, lock: &LockFile) {
    let json = serde_json::to_string_pretty(lock).expect("Failed to serialize lock");
    fs::write(path, json).expect("Failed to write mods.lock");
}
