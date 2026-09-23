//! Sync Workshop mods into `addons/` and maintain `mods.lock`.

mod apply;
mod audit;
mod identify;
mod missing;
mod plan;
mod report;
mod size;
mod updates;
mod verify;

#[cfg(test)]
mod tests;

use std::path::Path;

use crate::error::UksftaError;
use crate::lock::load_lock;
use crate::modlist::ModEntry;
use crate::steam::{find_all_workshop_caches, load_workshop_items};

pub use self::audit::audit;
pub use self::identify::identify;
pub use self::size::{lookup_sizes, missing_size_ids, modlist_size, size_share_colour};
pub use self::updates::check_updates;
pub use self::verify::verify;

/// Sync the requested mods from the Workshop caches into `addons/`, then
/// write the lock. With `dry_run`, print the diff and change nothing.
/// With `offline`, skip any network use, so dependency resolution is off.
pub fn sync_mods(
    mods: &[ModEntry],
    ignored: &[String],
    dry_run: bool,
    offline: bool,
    modlist: bool,
    modlist_path: &Path,
    resolve_deps: bool,
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

    // Load VDF metadata for timestamps from all caches
    let workshop_items = load_workshop_items();

    let plan = plan::build_plan(mods, ignored, &caches, &lock, &workshop_items);

    if dry_run {
        report::print_dry_run(&plan, modlist, modlist_path);
        report::print_summary(&plan);
        return Ok(());
    }

    let new_lock = apply::copy_pbos(&plan, &lock, addons_dir)?;
    report::print_synced(&plan);
    apply::remove_removed(&plan)?;
    apply::save_lock_if_changed(lock_path, &lock, &new_lock)?;

    missing::report_missing(
        mods,
        &caches,
        &plan,
        modlist,
        modlist_path,
        resolve_deps && !offline,
    )?;

    report::print_summary(&plan);
    Ok(())
}
