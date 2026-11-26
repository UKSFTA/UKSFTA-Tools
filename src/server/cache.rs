//! Read the local Steam Workshop cache. Signatures and a repair `meta.cpp`
//! come from there, never from the server.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::plan::Signature;
use crate::lock::LockFile;
use crate::modlist::ModEntry;
use crate::steam::{find_all_workshop_caches, find_mod_in_caches};

/// Environment override for the local Workshop cache. Use it for a Steam
/// library in a non-standard place, and for tests.
const CACHE_ENV: &str = "UKSFTA_WORKSHOP_CACHE";

pub fn workshop_caches() -> Vec<PathBuf> {
    match std::env::var_os(CACHE_ENV) {
        Some(value) => std::env::split_paths(&value).collect(),
        None => find_all_workshop_caches(),
    }
}

/// Workshop id to a `meta.cpp` in the local cache, copied verbatim when the
/// remote file must be repaired.
pub fn meta_sources(caches: &[PathBuf], mods: &[ModEntry]) -> HashMap<String, PathBuf> {
    let mut sources = HashMap::new();
    for entry in mods {
        if let Some(dir) = find_mod_in_caches(caches, &entry.id) {
            let path = dir.join("meta.cpp");
            if path.exists() {
                sources.insert(entry.id.clone(), path);
            }
        }
    }
    sources
}

/// Group the cached `.bisign` files by `<id>/<pbo>`.
pub fn signatures(caches: &[PathBuf], mods: &[ModEntry]) -> HashMap<String, Vec<Signature>> {
    let mut grouped: HashMap<String, Vec<Signature>> = HashMap::new();
    for entry in mods {
        let Some(dir) = find_mod_in_caches(caches, &entry.id) else {
            continue;
        };
        let Ok(items) = fs::read_dir(dir.join("addons")) else {
            continue;
        };
        for item in items.flatten() {
            let path = item.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(owner) = signature_owner(name) else {
                continue;
            };
            let Ok(meta) = item.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            grouped
                .entry(format!("{}/{}", entry.id, owner))
                .or_default()
                .push(Signature {
                    path: path.clone(),
                    size: meta.len(),
                    pbo_size: cached_pbo_size(&dir, owner),
                });
        }
    }
    for list in grouped.values_mut() {
        list.sort_by(|a, b| a.path.cmp(&b.path));
    }
    grouped
}

/// Lock path (for example `addons/CBA.pbo`) to the local size.
pub fn local_sizes(lock: &LockFile) -> HashMap<String, u64> {
    let mut sizes = HashMap::new();
    for entry in lock.mods.values() {
        for file in &entry.files {
            if let Ok(meta) = fs::metadata(file) {
                if meta.is_file() {
                    sizes.insert(file.clone(), meta.len());
                }
            }
        }
    }
    sizes
}

/// Report a locked PBO that is not in the local pack. The server check
/// cannot compare a file that is absent locally.
pub fn warn_missing_local(mods: &[ModEntry], lock: &LockFile) {
    for entry in mods {
        let Some(locked) = lock.mods.get(&entry.id) else {
            continue;
        };
        for file in &locked.files {
            if !Path::new(file).is_file() {
                eprintln!("Warning: {}: {file} is not in the local pack", entry.id);
            }
        }
    }
}

/// The PBO that a cached signature belongs to:
/// `ace_zeus.pbo.ace_3.21.3.bisign` -> `ace_zeus.pbo`.
fn signature_owner(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".bisign")?;
    let pos = stem.find(".pbo.")?;
    Some(&stem[..pos + 4])
}

/// The size of the cached PBO that a signature was made for. The pair lets
/// the plan skip a signature from a different PBO version.
fn cached_pbo_size(mod_dir: &Path, pbo: &str) -> Option<u64> {
    let meta = fs::metadata(mod_dir.join("addons").join(pbo)).ok()?;
    meta.is_file().then_some(meta.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_entry(id: &str) -> ModEntry {
        ModEntry {
            id: id.to_string(),
            name: "CBA_A3".to_string(),
            tags: Vec::new(),
            role: "mod".to_string(),
            enabled: true,
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn signature_owner_finds_the_pbo_name() {
        assert_eq!(
            signature_owner("ace_zeus.pbo.ace_3.21.3.115.bisign"),
            Some("ace_zeus.pbo")
        );
        assert_eq!(signature_owner("ace_zeus.pbo"), None);
        assert_eq!(signature_owner("other.bisign"), None);
    }

    #[test]
    fn signatures_record_the_size_of_their_pbo() {
        let dir = tempfile::TempDir::new().unwrap();
        let addons = dir.path().join("450814997/addons");
        fs::create_dir_all(&addons).unwrap();
        fs::write(addons.join("cba_common.pbo"), b"0123456789").unwrap();
        fs::write(addons.join("cba_common.pbo.cba_3.18.bisign"), b"sig").unwrap();
        fs::write(addons.join("orphan.pbo.key.bisign"), b"sig").unwrap();

        let map = signatures(&[dir.path().to_path_buf()], &[mod_entry("450814997")]);
        let paired = &map["450814997/cba_common.pbo"][0];
        assert_eq!(paired.size, 3);
        assert_eq!(paired.pbo_size, Some(10));

        let orphan = &map["450814997/orphan.pbo"][0];
        assert_eq!(orphan.pbo_size, None, "the cache holds no such PBO");
    }
}
