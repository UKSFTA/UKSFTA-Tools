use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::lock::{Dependency, LockFile, ModLockEntry};
use crate::modlist::ModEntry;
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
/// the cache, what to remove, and the status counts.
pub struct SyncPlan {
    pub planned: Vec<PlannedMod>,
    pub missing_from_cache: Vec<(String, String)>,
    pub removed: Vec<RemovedMod>,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
}

/// Classify each requested mod against the Workshop caches and the lock.
pub fn build_plan(
    mods: &[ModEntry],
    ignored: &[String],
    caches: &[PathBuf],
    lock: &LockFile,
    workshop_items: &HashMap<String, WorkshopItem>,
) -> SyncPlan {
    let addons_dir = Path::new("addons");
    let mut planned: Vec<PlannedMod> = Vec::new(); // (entry, id, status, source PBO paths)
    let mut missing_from_cache = Vec::new(); // (id, name)

    for entry in mods {
        // Find the mod in any cache
        let mod_path = match find_mod_in_caches(caches, &entry.id) {
            Some(p) => p,
            None => {
                missing_from_cache.push((entry.id.clone(), entry.name.clone()));
                continue;
            }
        };

        // Get metadata from mod.cpp/meta.cpp if available
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

        // Timestamp from ACF (Workshop's last update)
        let updated = workshop_items
            .get(&entry.id)
            .map(|item| item.time_updated.to_string())
            .unwrap_or_else(|| "0".to_string());

        let files: Vec<String> = pbos
            .iter()
            .map(|p| {
                addons_dir
                    .join(p.file_name().unwrap())
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        // Classify against lock
        let status = match lock.mods.get(&entry.id) {
            None => "added",
            Some(locked) => {
                let files_exist = locked.files.iter().all(|f| Path::new(f).exists());
                if locked.updated != updated || !files_exist {
                    "updated"
                } else {
                    "unchanged"
                }
            }
        };

        // Resolve declared dependency names from the mods list where possible
        let dependencies: Vec<Dependency> = entry
            .dependencies
            .iter()
            .map(|dep_id| Dependency {
                id: dep_id.clone(),
                name: mods
                    .iter()
                    .find(|m| &m.id == dep_id)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
            })
            .collect();

        planned.push(PlannedMod {
            entry: ModLockEntry {
                files,
                name: display_name,
                tags: entry.tags.clone(),
                dependencies,
                updated,
            },
            id: entry.id.clone(),
            status: status.to_string(),
            sources: pbos,
        });
    }

    // Removed: in lock but not in current mod list (and not ignored)
    let mut removed: Vec<RemovedMod> = Vec::new(); // (id, name, files)
    for (id, old_entry) in &lock.mods {
        if !mods.iter().any(|m| &m.id == id) && !ignored.contains(id) {
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
    }
}
