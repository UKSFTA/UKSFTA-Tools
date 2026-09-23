use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::plan::SyncPlan;
use crate::error::UksftaError;
use crate::modlist::{generate_modlist, print_dep_tree, resolve_transitive_deps, ModEntry};
use crate::util::workshop_url;

/// Warn about mods absent from the cache, resolve their dependencies when
/// requested, and generate the launcher modlist.
pub fn report_missing(
    mods: &[ModEntry],
    caches: &[PathBuf],
    plan: &SyncPlan,
    modlist: bool,
    modlist_path: &Path,
    resolve_deps: bool,
) -> Result<(), UksftaError> {
    let missing_from_cache = &plan.missing_from_cache;

    // Resolve dependencies once (only when requested) so warnings and
    // modlist both benefit without double-fetching Workshop pages.
    let (mods_for_list, deps_by_mod) = if resolve_deps {
        let known_ids: HashSet<String> = mods.iter().map(|m| m.id.clone()).collect();
        let cached_ids: HashSet<String> = caches
            .iter()
            .flat_map(|cache| {
                fs::read_dir(cache)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .filter_map(|e| e.file_name().into_string().ok())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect();
        resolve_transitive_deps(missing_from_cache, &known_ids, &cached_ids)
    } else {
        (missing_from_cache.clone(), std::collections::HashMap::new())
    };

    for (id, name) in missing_from_cache {
        eprintln!("Warning: {} ({}) not found in Workshop cache", name, id);
        if resolve_deps {
            let mut visited = HashSet::new();
            visited.insert(id.clone());
            print_dep_tree(id, &deps_by_mod, "         ", &mut visited);
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
            generate_modlist(&mods_for_list, modlist_path)?;
        }
    } else if modlist {
        println!("No missing mods — modlist not generated.");
    }

    Ok(())
}
