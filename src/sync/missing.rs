use std::collections::HashSet;
use std::path::Path;

use super::plan::SyncPlan;
use crate::error::UksftaError;
use crate::modlist::{generate_modlist, print_dep_tree};
use crate::util::workshop_url;

/// Warn about mods absent from the cache, print their dependency trees,
/// and generate the launcher modlist. No network access happens here.
pub fn report_missing(
    plan: &SyncPlan,
    modlist: bool,
    modlist_path: &Path,
) -> Result<(), UksftaError> {
    let missing_from_cache = &plan.missing_from_cache;

    for (id, name) in missing_from_cache {
        eprintln!("Warning: {} ({}) not found in Workshop cache", name, id);
        if plan.discovered_deps.contains_key(id) {
            let mut visited = HashSet::new();
            visited.insert(id.clone());
            print_dep_tree(id, &plan.discovered_deps, "         ", &mut visited);
        }
        eprintln!("         {}", workshop_url(id));
    }

    if !missing_from_cache.is_empty() {
        println!(
            "\nSubscribe to the {} missing mods in Steam:",
            missing_from_cache.len()
        );
        for (id, _) in missing_from_cache {
            println!("steam://url/CommunityFilePage/{}", id);
        }

        if modlist {
            generate_modlist(missing_from_cache, modlist_path)?;
        }
    } else if modlist {
        println!("No missing mods — modlist not generated.");
    }

    Ok(())
}
