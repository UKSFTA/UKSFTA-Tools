use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::plan::{classify_status, dest_files, PlannedMod};
use crate::lock::{LockFile, ModLockEntry};
use crate::modlist::{is_non_mod_app_id, DepMap, ModEntry};
use crate::pbo::get_mod_metadata;
use crate::steam::{find_mod_in_caches, WorkshopItem};
use crate::util::find_pbos;

/// Planned dependency mods, the dependencies absent from every cache, the
/// ids that must not be treated as removed, and the required dependency
/// ids that triggered the expansion.
pub(crate) struct Expansion {
    pub planned: Vec<PlannedMod>,
    pub missing: Vec<(String, String)>,
    pub wanted: HashSet<String>,
    pub required: HashSet<String>,
}

/// Expand every declared and discovered dependency into the synced set.
/// An id already known as a root, ignored, or a non-mod app id is skipped.
pub(crate) fn expand_dependencies(
    mods: &[ModEntry],
    ignored: &[String],
    caches: &[PathBuf],
    lock: &LockFile,
    workshop_items: &HashMap<String, WorkshopItem>,
    discovered: &DepMap,
) -> Expansion {
    let known: HashSet<String> = mods.iter().map(|m| m.id.clone()).collect();
    let ignored_set: HashSet<String> = ignored.iter().cloned().collect();

    let mut dep_ids: BTreeSet<String> = BTreeSet::new();
    for entry in mods {
        for id in &entry.dependencies {
            dep_ids.insert(id.clone());
        }
    }
    for deps in discovered.values() {
        for (id, _) in deps {
            dep_ids.insert(id.clone());
        }
    }
    dep_ids.retain(|id| !known.contains(id) && !ignored_set.contains(id) && !is_non_mod_app_id(id));

    let mut discovered_names: HashMap<String, String> = HashMap::new();
    for deps in discovered.values() {
        for (id, name) in deps {
            discovered_names
                .entry(id.clone())
                .or_insert_with(|| name.clone());
        }
    }

    let mut planned = Vec::new();
    let mut missing = Vec::new();

    for dep_id in &dep_ids {
        let fallback = || {
            discovered_names
                .get(dep_id)
                .cloned()
                .unwrap_or_else(|| format!("Mod {dep_id}"))
        };

        let Some(mod_path) = find_mod_in_caches(caches, dep_id) else {
            missing.push((dep_id.clone(), fallback()));
            continue;
        };

        let mod_meta = get_mod_metadata(&mod_path);
        let name = if mod_meta.name.is_empty() {
            fallback()
        } else {
            mod_meta.name
        };

        let pbos = find_pbos(&mod_path);
        if pbos.is_empty() {
            missing.push((dep_id.clone(), name));
            continue;
        }

        let updated = workshop_items
            .get(dep_id)
            .map(|item| item.time_updated.to_string())
            .unwrap_or_else(|| "0".to_string());
        let status = classify_status(lock, dep_id, &updated).to_string();

        planned.push(PlannedMod {
            entry: ModLockEntry {
                files: dest_files(Path::new("addons"), &pbos),
                name,
                tags: Vec::new(),
                dependencies: Vec::new(),
                updated,
            },
            id: dep_id.clone(),
            status,
            sources: pbos,
        });
    }

    let mut wanted: HashSet<String> = known.clone();
    wanted.extend(dep_ids.iter().cloned());
    let required: HashSet<String> = dep_ids.into_iter().collect();

    Expansion {
        planned,
        missing,
        wanted,
        required,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::LockFile;

    fn root_entry() -> ModEntry {
        ModEntry {
            id: "9999999999".to_string(),
            name: "Root".to_string(),
            tags: Vec::new(),
            role: "mod".to_string(),
            enabled: true,
            dependencies: vec!["450814997".to_string()],
        }
    }

    fn cache_with_dep() -> (std::path::PathBuf, std::path::PathBuf) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        // Each call gets its own directory: the two tests that use this
        // helper run in parallel and must not share a cache.
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("uksfta-expand-{}-{}", std::process::id(), seq));
        let cache = root.join("107410");
        let dep_dir = cache.join("450814997");
        std::fs::create_dir_all(&dep_dir).unwrap();
        std::fs::write(dep_dir.join("x.pbo"), b"pbo").unwrap();
        (root, cache)
    }

    #[test]
    fn expansion_repacks_cached_dependency() {
        let (root, cache) = cache_with_dep();
        let mods = vec![root_entry()];
        let lock = LockFile {
            version: 1,
            mods: HashMap::new(),
        };

        let expansion = expand_dependencies(
            &mods,
            &[],
            std::slice::from_ref(&cache),
            &lock,
            &HashMap::new(),
            &DepMap::new(),
        );

        assert_eq!(expansion.planned.len(), 1);
        assert_eq!(expansion.planned[0].id, "450814997");
        assert_eq!(expansion.planned[0].status, "added");
        assert!(!expansion.planned[0].entry.files.is_empty());
        assert!(expansion.required.contains("450814997"));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn expansion_does_not_repack_ignored_dependency() {
        let (root, cache) = cache_with_dep();
        let mods = vec![root_entry()];
        let lock = LockFile {
            version: 1,
            mods: HashMap::new(),
        };

        let expansion = expand_dependencies(
            &mods,
            &["450814997".to_string()],
            std::slice::from_ref(&cache),
            &lock,
            &HashMap::new(),
            &DepMap::new(),
        );

        assert!(expansion.planned.is_empty());
        assert!(expansion.missing.is_empty());

        std::fs::remove_dir_all(&root).ok();
    }
}
