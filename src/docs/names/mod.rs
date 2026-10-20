//! Resolve missing Workshop titles from the Steam Web API. The lock file
//! stores `Mod <id>` as a placeholder when a name was never discovered, so
//! only those placeholders and blank names are fetched. Resolved titles are
//! cached so later offline runs keep them.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::error::UksftaError;
use crate::lock::LockFile;
use crate::workshop_api::{build_client, fetch_published_file_details};

/// Ids that have no usable name in the lock file, sorted and de-duplicated.
pub fn missing_name_ids(lock: &LockFile) -> Vec<String> {
    let mut ids = HashSet::new();
    for (id, entry) in &lock.mods {
        if needs_name(&entry.name, id) {
            ids.insert(id.clone());
        }
        for dependency in &entry.dependencies {
            if needs_name(&dependency.name, &dependency.id) {
                ids.insert(dependency.id.clone());
            }
        }
    }
    let mut ids: Vec<String> = ids.into_iter().collect();
    ids.sort();
    ids
}

/// Replace placeholder names with the titles in `names`. Real names and ids
/// absent from the map are left alone. Returns the number of names replaced.
pub fn apply_names(lock: &mut LockFile, names: &HashMap<String, String>) -> usize {
    let mut count = 0;
    for (id, entry) in lock.mods.iter_mut() {
        if let Some(title) = names.get(id) {
            if needs_name(&entry.name, id) {
                entry.name = title.clone();
                count += 1;
            }
        }
        for dependency in entry.dependencies.iter_mut() {
            if let Some(title) = names.get(&dependency.id) {
                if needs_name(&dependency.name, &dependency.id) {
                    dependency.name = title.clone();
                    count += 1;
                }
            }
        }
    }
    count
}

/// Read the cache. A missing file is empty; an unparseable file is an error.
pub fn load_cache(path: &Path) -> Result<HashMap<String, String>, UksftaError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|e| UksftaError::Parse(format!("failed to parse {}: {e}", path.display())))
}

/// Write the cache, creating the parent directory on first use.
pub fn save_cache(path: &Path, names: &HashMap<String, String>) -> Result<(), UksftaError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(names)
        .map_err(|e| UksftaError::Parse(format!("failed to serialize name cache: {e}")))?;
    crate::atomic::write_atomic(path, json.as_bytes())
}

/// Fetch titles for ids that are not cached yet. With `refresh`, cached ids
/// are fetched again so a changed Workshop title is picked up. New titles
/// are added to `cache`.
pub fn resolve_missing(
    lock: &LockFile,
    cache: &mut HashMap<String, String>,
    refresh: bool,
) -> Result<usize, UksftaError> {
    let ids = ids_to_fetch(lock, cache, refresh);
    if ids.is_empty() {
        return Ok(0);
    }
    let details = fetch_published_file_details(&ids, &build_client())?;
    let mut count = 0;
    for detail in details {
        if detail.result == 1 && !detail.title.trim().is_empty() {
            cache.insert(detail.publishedfileid, detail.title);
            count += 1;
        }
    }
    Ok(count)
}

/// Ids that a lookup would request. Only names that are still placeholders
/// are considered, because real lock names are never displayed from the
/// cache.
pub fn ids_to_fetch(
    lock: &LockFile,
    cache: &HashMap<String, String>,
    refresh: bool,
) -> Vec<String> {
    missing_name_ids(lock)
        .into_iter()
        .filter(|id| refresh || !cache.contains_key(id))
        .collect()
}

/// Drop cached ids that no longer appear in the lock. Returns the number
/// removed.
pub fn prune_cache(cache: &mut HashMap<String, String>, lock: &LockFile) -> usize {
    let keep = lock_ids(lock);
    let before = cache.len();
    cache.retain(|id, _| keep.contains(id));
    before - cache.len()
}

fn lock_ids(lock: &LockFile) -> HashSet<String> {
    let mut ids = HashSet::new();
    for (id, entry) in &lock.mods {
        ids.insert(id.clone());
        for dependency in &entry.dependencies {
            ids.insert(dependency.id.clone());
        }
    }
    ids
}

fn needs_name(name: &str, id: &str) -> bool {
    name.trim().is_empty() || name == format!("Mod {id}")
}

#[cfg(test)]
mod tests;
