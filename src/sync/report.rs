use crate::util::workshop_url;
use std::path::Path;

use super::plan::SyncPlan;

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
