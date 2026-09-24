use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::expand::expand_dependencies;
use crate::lock::{Dependency, LockFile, ModLockEntry};
use crate::modlist::{DepMap, ModEntry};
use crate::pbo::get_mod_metadata;
use crate::steam::{find_mod_in_caches, WorkshopItem};
use crate::util::find_pbos;

/// One mod classified against the lock: its lock entry, Workshop id,
/// status (added / updated / unchanged), and source PBO paths.
pub struct PlannedMod {
    pub entry: ModLockEntry,
    pub id: String,
    pub status: String,
    pub sources: Vec<PathBuf>,
}

/// A mod in the lock that is no longer in the current list.
pub struct RemovedMod {
    pub id: String,
    pub name: String,
    pub files: Vec<String>,
}

/// The classified outcome of a sync: what to copy, what is missing from
/// the cache, what to remove, the status counts, the discovered
/// dependency map, and the required dependency ids.
pub struct SyncPlan {
    pub planned: Vec<PlannedMod>,
    pub missing_from_cache: Vec<(String, String)>,
    pub removed: Vec<RemovedMod>,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub discovered_deps: DepMap,
    pub required_deps: HashSet<String>,
}

/// Classify a mod against the lock: added, updated, or unchanged.
pub(crate) fn classify_status(lock: &LockFile, id: &str, updated: &str) -> &'static str {
    match lock.mods.get(id) {
        None => "added",
        Some(locked) => {
            let files_exist = locked.files.iter().all(|f| Path::new(f).exists());
            if locked.updated != updated || !files_exist {
                "updated"
            } else {
                "unchanged"
            }
        }
    }
}

/// Map source PBO paths to their destination paths under `addons_dir`.
pub(crate) fn dest_files(addons_dir: &Path, pbos: &[PathBuf]) -> Vec<String> {
    pbos.iter()
        .filter_map(|p| p.file_name())
        .map(|name| addons_dir.join(name).to_string_lossy().to_string())
        .collect()
}

/// The display name for a dependency id: a known mod name first, then a
/// discovered name, then an empty string.
fn dependency_name(
    mods: &[ModEntry],
    discovered_names: &HashMap<String, String>,
    dep_id: &str,
) -> String {
    mods.iter()
        .find(|m| m.id == dep_id)
        .map(|m| m.name.clone())
        .or_else(|| discovered_names.get(dep_id).cloned())
        .unwrap_or_default()
}

/// Classify each requested mod against the Workshop caches and the lock,
/// then expand every declared and discovered dependency.
pub fn build_plan(
    mods: &[ModEntry],
    ignored: &[String],
    caches: &[PathBuf],
    lock: &LockFile,
    workshop_items: &HashMap<String, WorkshopItem>,
    discovered: &DepMap,
) -> SyncPlan {
    let addons_dir = Path::new("addons");
    let mut planned: Vec<PlannedMod> = Vec::new();
    let mut missing_from_cache = Vec::new();

    for entry in mods {
        let mod_path = match find_mod_in_caches(caches, &entry.id) {
            Some(p) => p,
            None => {
                missing_from_cache.push((entry.id.clone(), entry.name.clone()));
                continue;
            }
        };

        let mod_meta = get_mod_metadata(&mod_path);
        let display_name = if mod_meta.name.is_empty() {
            entry.name.clone()
        } else {
            mod_meta.name
        };

        let pbos = find_pbos(&mod_path);
        if pbos.is_empty() {
            missing_from_cache.push((entry.id.clone(), entry.name.clone()));
            continue;
        }

        let updated = workshop_items
            .get(&entry.id)
            .map(|item| item.time_updated.to_string())
            .unwrap_or_else(|| "0".to_string());
        let status = classify_status(lock, &entry.id, &updated).to_string();

        planned.push(PlannedMod {
            entry: ModLockEntry {
                files: dest_files(addons_dir, &pbos),
                name: display_name,
                tags: entry.tags.clone(),
                dependencies: Vec::new(),
                updated,
            },
            id: entry.id.clone(),
            status,
            sources: pbos,
        });
    }

    // Names for dependency ids that are not themselves known roots.
    let mut discovered_names: HashMap<String, String> = HashMap::new();
    for deps in discovered.values() {
        for (id, name) in deps {
            discovered_names
                .entry(id.clone())
                .or_insert_with(|| name.clone());
        }
    }

    // Fill each root's lock dependency names now that discovery has run.
    for item in &mut planned {
        let Some(entry) = mods.iter().find(|m| m.id == item.id) else {
            continue;
        };
        item.entry.dependencies = entry
            .dependencies
            .iter()
            .map(|dep_id| Dependency {
                id: dep_id.clone(),
                name: dependency_name(mods, &discovered_names, dep_id),
            })
            .collect();
    }

    let expansion = expand_dependencies(mods, ignored, caches, lock, workshop_items, discovered);
    planned.extend(expansion.planned);
    missing_from_cache.extend(expansion.missing);

    let ignored_set: HashSet<String> = ignored.iter().cloned().collect();
    let mut removed: Vec<RemovedMod> = Vec::new();
    for (id, old_entry) in &lock.mods {
        if !expansion.wanted.contains(id) && !ignored_set.contains(id) {
            removed.push(RemovedMod {
                id: id.clone(),
                name: old_entry.name.clone(),
                files: old_entry.files.clone(),
            });
        }
    }

    let added = planned.iter().filter(|p| p.status == "added").count();
    let updated = planned.iter().filter(|p| p.status == "updated").count();
    let unchanged = planned.iter().filter(|p| p.status == "unchanged").count();

    SyncPlan {
        planned,
        missing_from_cache,
        removed,
        added,
        updated,
        unchanged,
        discovered_deps: discovered.clone(),
        required_deps: expansion.required,
    }
}
