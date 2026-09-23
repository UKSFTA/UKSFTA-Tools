//! The local identity cache: search term to confirmed Workshop candidates.

use std::fs;
use std::path::{Path, PathBuf};

use super::model::ScoredCandidate;
use crate::origin::IdentityCache;

/// The local identity cache: maps a PBO search term to the confirmed
/// (id, title) pairs found for it. Persisted to .uksfta/identities.json
/// so repeat investigations reuse prior searches entirely offline. This
/// file is gitignored and never leaves the machine.
fn identity_cache_path() -> PathBuf {
    Path::new(".uksfta").join("identities.json")
}

pub(crate) fn load_identity_cache() -> IdentityCache<ScoredCandidate> {
    let path = identity_cache_path();
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return std::collections::HashMap::new(),
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub(crate) fn save_identity_cache(cache: &IdentityCache<ScoredCandidate>) {
    let path = identity_cache_path();
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(cache) {
        // Write through a temp file and a rename, so a killed run never
        // leaves a truncated cache that forces a full re-search next time.
        let _ = crate::atomic::write_atomic(&path, json.as_bytes());
    }
}
