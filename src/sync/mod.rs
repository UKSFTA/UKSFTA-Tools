//! Sync Workshop mods into `addons/` and maintain `mods.lock`.

mod apply;
mod audit;
mod expand;
mod fill;
mod identify;
mod missing;
mod plan;
mod report;
mod size;
mod updates;
mod verify;

#[cfg(test)]
mod tests;

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::error::UksftaError;
use crate::lock::load_lock;
use crate::modlist::{
    persist_dependencies, persist_ignored, resolve_all_required, DepMap, ModEntry,
};
use crate::steam::{find_all_workshop_caches, load_workshop_items};
use crate::workshop_api::build_client;

pub use self::audit::audit;
pub use self::identify::identify;
pub use self::size::{lookup_sizes, missing_size_ids, modlist_size, size_share_colour};
pub use self::updates::check_updates;
pub use self::verify::verify;

/// Options for one sync run.
pub struct SyncOptions<'a> {
    pub sources_path: &'a Path,
    pub dry_run: bool,
    pub offline: bool,
    pub modlist: bool,
    pub modlist_path: &'a Path,
    pub resolve_deps: bool,
    pub fill_from: Option<&'a Path>,
}

/// Merge each root's discovered closure into its declared dependencies.
/// Returns the roots whose value changed, as (id, sorted deps).
fn merge_dependencies(mods: &mut [ModEntry], by_root: &DepMap) -> Vec<(String, Vec<String>)> {
    let mut updates = Vec::new();
    for entry in mods.iter_mut() {
        let Some(deps) = by_root.get(&entry.id) else {
            continue;
        };
        let mut merged: BTreeSet<String> = entry.dependencies.iter().cloned().collect();
        for (id, _) in deps {
            merged.insert(id.clone());
        }
        let merged: Vec<String> = merged.into_iter().collect();
        if merged != entry.dependencies {
            entry.dependencies = merged.clone();
            updates.push((entry.id.clone(), merged));
        }
    }
    updates
}

/// Required dependency ids that are absent from every Workshop cache.
/// A root mod absent from the cache is not a dependency and stays
/// warn-only.
pub(crate) fn unpullable_dependencies(plan: &plan::SyncPlan) -> Vec<String> {
    plan.missing_from_cache
        .iter()
        .map(|(id, _)| id.clone())
        .filter(|id| plan.required_deps.contains(id))
        .collect()
}

/// The Workshop ids a launcher modlist provides. Direct ids always count.
/// With network access, ids that name a Workshop collection also contribute
/// their members. A failed collection lookup is a warning and is skipped.
fn provided_modlist_ids(path: &Path, offline: bool) -> Result<HashSet<String>, UksftaError> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        UksftaError::Input(format!("failed to read modlist {}: {e}", path.display()))
    })?;
    let mut provided = fill::provided_ids(&content);
    if offline {
        return Ok(provided);
    }

    let direct: Vec<String> = provided.iter().cloned().collect();
    let client = build_client();
    for id in direct {
        match crate::workshop_api::fetch_collection_members(&id, &client) {
            Ok(members) if !members.is_empty() => {
                println!("Collection {} provides {} member mod(s)", id, members.len());
                fill::merge_members(&mut provided, &members);
            }
            Ok(_) => {}
            Err(e) => eprintln!("Warning: failed to expand collection {}: {}", id, e),
        }
    }
    Ok(provided)
}

/// Sync the requested mods from the Workshop caches into `addons/`, then
/// write the lock. With `dry_run`, print the diff and change nothing.
/// With `offline`, skip any network use, so dependency resolution is off.
pub fn sync_mods(
    mods: &[ModEntry],
    ignored: &[String],
    opts: &SyncOptions,
) -> Result<(), UksftaError> {
    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        return Err(UksftaError::Input(
            "Workshop cache not found. Is Steam installed?".to_string(),
        ));
    }
    let addons_dir = Path::new("addons");
    let lock_path = Path::new("mods.lock");

    let lock = load_lock(lock_path)?;
    let workshop_items = load_workshop_items();

    let mut planned_mods = mods.to_vec();
    let mut discovered: DepMap = DepMap::new();
    let mut ignored: Vec<String> = ignored.to_vec();

    if opts.resolve_deps && !opts.offline {
        let known: HashSet<String> = planned_mods.iter().map(|m| m.id.clone()).collect();
        let ignored_set: HashSet<String> = ignored.iter().cloned().collect();
        let (_all, by_root) = resolve_all_required(&planned_mods, &known, &ignored_set);
        report::print_discovered_deps(&planned_mods, &by_root, &caches);
        let updates = merge_dependencies(&mut planned_mods, &by_root);
        let should_persist = !opts.dry_run && !updates.is_empty();
        if should_persist && persist_dependencies(opts.sources_path, &updates)? {
            println!(
                "Persisted {} dependency link(s) to mod_sources.txt",
                updates.len()
            );
        }
        discovered = by_root;

        // A launcher modlist the user already runs can provide dependencies
        // that must be tracked but not repacked into addons/.
        if let Some(fill_path) = opts.fill_from {
            let provided = provided_modlist_ids(fill_path, opts.offline)?;
            let satisfied = fill::satisfied_dependencies(&planned_mods, &discovered, &provided);
            report::print_satisfied_deps(&satisfied);
            if !opts.dry_run && !satisfied.is_empty() {
                let entries = fill::ignore_entries(&satisfied);
                if persist_ignored(opts.sources_path, &entries)? {
                    println!(
                        "Persisted {} provided dependency(ies) as ignore in mod_sources.txt",
                        entries.len()
                    );
                }
            }
            ignored.extend(fill::ignore_ids(&satisfied));
        }
    }

    let plan = plan::build_plan(
        &planned_mods,
        &ignored,
        &caches,
        &lock,
        &workshop_items,
        &discovered,
    );

    // A required dependency absent from the cache must stop the run before
    // any PBO is copied, so a broken mod is never published.
    let unpullable = unpullable_dependencies(&plan);
    if !unpullable.is_empty() {
        missing::report_missing(&plan, opts.modlist && !opts.dry_run, opts.modlist_path)?;
        return Err(UksftaError::Check(format!(
            "required dependencies missing from the Workshop cache: {}",
            unpullable.join(", ")
        )));
    }

    if opts.dry_run {
        report::print_dry_run(&plan, opts.modlist, opts.modlist_path);
        report::print_summary(&plan);
        return Ok(());
    }

    let new_lock = apply::copy_pbos(&plan, &lock, addons_dir)?;
    report::print_synced(&plan);
    apply::remove_removed(&plan)?;
    apply::save_lock_if_changed(lock_path, &lock, &new_lock)?;

    missing::report_missing(&plan, opts.modlist, opts.modlist_path)?;

    report::print_summary(&plan);
    Ok(())
}
