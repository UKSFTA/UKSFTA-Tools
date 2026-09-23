//! Trace the Workshop origin of untracked PBOs in addons/.

mod api;
mod bikey;
mod cache;
mod changelog;
mod group;
mod model;
mod online;
mod online_group;
mod scoring;
mod search;
mod text;
mod visibility;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use self::cache::load_identity_cache;
use self::online::search_online;
use crate::error::UksftaError;
use crate::lock::load_lock;
use crate::origin::{
    build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id, ResolvedOrigin,
};
use crate::pbo::{
    get_mod_metadata, pbo_config_identity, pbo_identity, pbo_prefix, search_term_from_prefix,
};
use crate::steam::find_all_workshop_caches;
use crate::util::{paint, sanitize_for_terminal, stdout_is_tty, workshop_url, C_GREEN, C_RED};

pub use self::api::urlencode_pairs;

/// Print the resolved identity inventory from the local cache.
/// For each untracked PBO in addons/, derive its search term (same logic
/// as the online search) and show the cached candidate mods, if any.
/// No network: this reads only .uksfta/identities.json.
pub fn investigate_report() -> Result<(), UksftaError> {
    let addons_dir = Path::new("addons");
    if !addons_dir.exists() {
        return Err(UksftaError::Input("No addons/ directory".to_string()));
    }

    let cache = load_identity_cache();
    if cache.is_empty() {
        println!("No cached identities found. Run 'uksfta investigate --online' first.");
        return Ok(());
    }

    println!("Resolved identity inventory (from local cache):");
    let mut shown = 0;
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "pbo").unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                let term = pbo_config_identity(&path)
                    .or_else(|| pbo_identity(&path))
                    .or_else(|| pbo_prefix(&path))
                    .unwrap_or_else(|| name.clone());
                let term = search_term_from_prefix(&term);
                if let Some(hits) = cache.get(&term) {
                    if !hits.is_empty() {
                        let best = &hits[0];
                        println!(
                            "  {:<45} -> {} ({}) [term \"{}\"]",
                            name,
                            sanitize_for_terminal(&best.title),
                            best.id,
                            term
                        );
                        shown += 1;
                    }
                }
            }
        }
    }
    if shown == 0 {
        println!("  (no cached identities match the PBOs in addons/)");
    } else {
        println!("\n{} PBO(s) have cached identity candidates.", shown);
    }
    Ok(())
}

/// Trace the Workshop origin of untracked PBOs in addons/.
pub fn investigate(all: bool, online: bool) -> Result<(), UksftaError> {
    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        return Err(UksftaError::Input("Workshop cache not found".to_string()));
    }
    let addons_dir = Path::new("addons");
    if !addons_dir.exists() {
        return Err(UksftaError::Input("No addons/ directory".to_string()));
    }

    let mut mod_dirs = build_mod_dirs(&caches);

    // If the current directory is itself a Workshop mod folder (e.g. we are
    // investigating a pack's own contents), exclude it as a candidate.
    if let Some(self_id) = self_workshop_id(&caches) {
        println!(
            "Investigating Workshop mod {} — excluding it as a candidate.",
            self_id
        );
        mod_dirs.retain(|d| d.id != self_id);
    }

    // Tracked PBO filenames: from mods.lock if present, else all are untracked
    let lock_path = Path::new("mods.lock");
    let lock = load_lock(lock_path)?;
    let tracked: HashSet<String> = lock
        .mods
        .values()
        .flat_map(|m| m.files.iter())
        .filter_map(|f| {
            Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .collect();

    let mut results: Vec<(String, Option<ResolvedOrigin>)> = Vec::new();

    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "pbo").unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                if !all && tracked.contains(&name) {
                    continue;
                }
                results.push((name, None));
            }
        }
    }

    if results.is_empty() {
        println!("No untracked PBOs in addons/.");
        return Ok(());
    }

    // Build a single-pass index of every cached PBO name to its folders
    let index = build_pbo_index(&mod_dirs);

    // Match each untracked PBO against the index and arbitrate the origin
    for (pbo_name, origin) in &mut results {
        let pbo_path = addons_dir.join(pbo_name.as_str());
        let target_prefix = pbo_prefix(&pbo_path);
        let candidates = index
            .get(pbo_name.as_str())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        *origin = resolve_pbo_from_index(&pbo_path, candidates, target_prefix.as_deref());
    }

    // Report
    let use_colour = stdout_is_tty();
    println!("Untracked PBO Investigation:");
    println!("  {} PBO(s) examined", results.len());
    let mut unknown_count = 0;
    for (pbo, origin) in &results {
        let Some(origin) = origin else {
            unknown_count += 1;
            println!("  {} {}", paint(use_colour, C_RED, "[UNKNOWN]"), pbo);
            continue;
        };
        let id = &origin.id;
        let meta = mod_dirs
            .iter()
            .find(|d| &d.id == id)
            .map(|d| get_mod_metadata(&d.path))
            .unwrap_or_default();
        let name = if meta.name.is_empty() {
            format!("Mod {}", id)
        } else {
            meta.name
        };
        if origin.is_pack {
            println!(
                "  {} {} -> {} ({}) {} [pack: prefix {} — verify source]",
                paint(use_colour, C_GREEN, "[PACK]"),
                pbo,
                name,
                id,
                workshop_url(id),
                origin.prefix.as_deref().unwrap_or("unknown")
            );
        } else {
            println!(
                "  {} {} -> {} ({}) {}",
                paint(use_colour, C_GREEN, "[RESOLVED]"),
                pbo,
                name,
                id,
                workshop_url(id)
            );
        }
    }
    if unknown_count > 0 {
        println!(
            "\n{} PBO(s) could not be matched to any Workshop cache entry.",
            unknown_count
        );
        println!("They may come from a deleted or private mod, or be manually placed.");
    }

    if online {
        search_online(&results, addons_dir, use_colour);
    }
    Ok(())
}
