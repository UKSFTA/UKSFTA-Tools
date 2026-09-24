use crate::util::workshop_url;
use std::path::{Path, PathBuf};

use super::plan::SyncPlan;
use crate::modlist::{DepMap, ModEntry};
use crate::sync::fill::SatisfiedDep;

/// Print one line per dependency the launcher modlist satisfies. Those
/// dependencies are tracked as ignored and are not repacked.
pub fn print_satisfied_deps(satisfied: &[SatisfiedDep]) {
    for s in satisfied {
        eprintln!(
            "Dependency {} ({}) required by {} ({}): provided by the modlist, tracked as ignore",
            s.name, s.id, s.required_by_name, s.required_by
        );
    }
}

/// Print the dry-run diff preview and the missing-mod list.
pub fn print_dry_run(plan: &SyncPlan, modlist: bool, modlist_path: &Path) {
    println!("--- Diff preview (dry-run) ---");
    for item in &plan.planned {
        match item.status.as_str() {
            "added" => {
                println!("  [ADD]     {} ({})", item.entry.name, item.id);
                for f in &item.entry.files {
                    println!(
                        "    + {}",
                        Path::new(f).file_name().unwrap().to_string_lossy()
                    );
                }
            }
            "updated" => {
                println!("  [UPDATE]  {} ({})", item.entry.name, item.id);
                for f in &item.entry.files {
                    println!(
                        "    ~ {}",
                        Path::new(f).file_name().unwrap().to_string_lossy()
                    );
                }
            }
            _ => println!(
                "  [OK]      {} ({}) — {} PBOs present",
                item.entry.name,
                item.id,
                item.entry.files.len()
            ),
        }
    }
    for r in &plan.removed {
        println!("  [REMOVE]  {} ({})", r.name, r.id);
        for f in &r.files {
            println!(
                "    - {}",
                Path::new(f).file_name().unwrap().to_string_lossy()
            );
        }
    }
    for (id, name) in &plan.missing_from_cache {
        println!("  [MISSING] {} ({}) not found in Workshop cache", name, id);
        println!("            {}", workshop_url(id));
    }
    if modlist && !plan.missing_from_cache.is_empty() {
        println!("\nModlist would be written to {}", modlist_path.display());
    }
}

/// Print the applied-change summary.
pub fn print_synced(plan: &SyncPlan) {
    println!(
        "\nSynced: {} added, {} updated, {} unchanged, {} removed",
        plan.added,
        plan.updated,
        plan.unchanged,
        plan.removed.len()
    );
}

/// Print the final summary, shared by the dry-run and applied paths.
pub fn print_summary(plan: &SyncPlan) {
    println!(
        "\nSummary: {} added, {} updated, {} unchanged, {} removed",
        plan.added,
        plan.updated,
        plan.unchanged,
        plan.removed.len()
    );
}

/// Print one line per discovered dependency, to stderr so it shows in
/// both the dry-run and the applied run. The line states whether the
/// dependency is already cached or must be added to the missing list.
pub fn print_discovered_deps(roots: &[ModEntry], discovered: &DepMap, caches: &[PathBuf]) {
    for (root_id, deps) in discovered {
        if deps.is_empty() {
            continue;
        }
        let root_name = roots
            .iter()
            .find(|m| &m.id == root_id)
            .map(|m| m.name.as_str())
            .unwrap_or(root_id);
        for (dep_id, dep_name) in deps {
            if crate::steam::find_mod_in_caches(caches, dep_id).is_some() {
                eprintln!(
                    "Dependency {} ({}) required by {} ({}): repacking into addons/",
                    dep_name, dep_id, root_name, root_id
                );
            } else {
                eprintln!(
                    "Dependency {} ({}) required by {} ({}): not in Workshop cache, added to missing list",
                    dep_name, dep_id, root_name, root_id
                );
            }
        }
    }
}
