use std::collections::HashMap;
use std::fs;
use std::path::Path;

use super::plan::{PlannedMod, SyncPlan};
use crate::error::UksftaError;
use crate::lock::{default_lock_version, save_lock, LockFile};

/// Copy each changed mod's PBOs into `addons_dir` and build the new lock.
///
/// Entries for unchanged mods and mods still wanted but absent from the
/// cache are carried over from the previous lock.
pub fn copy_pbos(
    plan: &SyncPlan,
    lock: &LockFile,
    addons_dir: &Path,
) -> Result<LockFile, UksftaError> {
    let mut new_lock = LockFile {
        version: default_lock_version(),
        mods: HashMap::new(),
    };

    let work_items: Vec<&PlannedMod> = plan
        .planned
        .iter()
        .filter(|p| p.status != "unchanged")
        .collect();

    let bar = indicatif::ProgressBar::new(work_items.len() as u64);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} mods {msg}",
        )
        .unwrap()
        .progress_chars("##-"),
    );

    for item in work_items {
        bar.set_message(format!("{} ({})", item.entry.name, item.id));

        fs::create_dir_all(addons_dir)?;
        // entry.files[i] is the dest path, sources[i] is the real source path
        for (dest, src) in item.entry.files.iter().zip(item.sources.iter()) {
            fs::copy(src, dest)?;
        }
        new_lock.mods.insert(item.id.clone(), item.entry.clone());
        bar.inc(1);
    }
    bar.finish_and_clear();

    for item in &plan.planned {
        if item.status == "unchanged" {
            // Keep the existing lock entry as-is
            new_lock
                .mods
                .insert(item.id.clone(), lock.mods[&item.id].clone());
        }
    }

    // Preserve lock entries for mods still in the list but missing from
    // the Workshop cache. They are wanted but not currently syncable, so
    // their previous file records must not be dropped.
    for (id, _) in &plan.missing_from_cache {
        if let Some(entry) = lock.mods.get(id) {
            new_lock.mods.insert(id.clone(), entry.clone());
        }
    }

    Ok(new_lock)
}

/// Remove PBOs for mods no longer in the list.
pub fn remove_removed(plan: &SyncPlan) -> Result<(), UksftaError> {
    for r in &plan.removed {
        for file in &r.files {
            if Path::new(file).exists() {
                fs::remove_file(file)?;
            }
        }
        println!("Removed: {} ({})", r.name, r.id);
    }
    Ok(())
}

/// Write the lock only when its content actually changed. This avoids
/// touching the file (and dirtying the working tree) when nothing was
/// synced or removed.
pub fn save_lock_if_changed(
    lock_path: &Path,
    lock: &LockFile,
    new_lock: &LockFile,
) -> Result<(), UksftaError> {
    if new_lock.mods != lock.mods {
        save_lock(lock_path, new_lock)?;
    }
    Ok(())
}
