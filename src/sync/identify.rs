use std::path::Path;

use crate::error::UksftaError;
use crate::origin::{build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id};
use crate::pbo::pbo_prefix;
use crate::steam::find_all_workshop_caches;
use crate::util::{find_pbos, sanitize_for_terminal};
use crate::workshop_manifest::{shared_suffix, Confirmation, WorkshopIndex};

/// Show which PBOs in addons/ came from which Workshop mod.
pub fn identify() -> Result<(), UksftaError> {
    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        return Err(UksftaError::Input("Workshop cache not found".to_string()));
    }

    let addons_dir = Path::new("addons");
    if !addons_dir.exists() {
        return Err(UksftaError::Input("No addons/ directory".to_string()));
    }

    let mut mod_dirs = build_mod_dirs(&caches);

    // If the current directory is itself a Workshop mod folder, exclude it.
    if let Some(self_id) = self_workshop_id(&caches) {
        mod_dirs.retain(|d| d.id != self_id);
    }

    println!("PBO Origins:");
    let index = build_pbo_index(&mod_dirs);

    // The depot manifest index holds the exact bytes of every installed
    // Workshop file. A match confirms the item and the published path with
    // no network. A miss keeps the prefix result, which is the only signal
    // for a repacked file.
    let manifest_index = WorkshopIndex::load();

    for pbo in find_pbos(addons_dir) {
        let Some(name) = pbo.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let target_prefix = pbo_prefix(&pbo);
        let candidates = index.candidates(&name);
        let prefix_origin = resolve_pbo_from_index(&pbo, candidates, target_prefix.as_deref());

        // Discovery runs on the basename and size; only a name match is
        // worth hashing. The hash then confirms the item, never the name.
        if let Some(index) = manifest_index.as_ref() {
            let candidates = index.candidates_for(&pbo);
            if !candidates.is_empty() {
                match index.confirmation(&pbo) {
                    Confirmation::Match(entries) => {
                        let hint = prefix_origin.as_ref().map(|o| o.id.as_str());
                        let chosen = index
                            .pick(entries, hint)
                            .expect("a confirmed match has at least one entry");
                        let shared = shared_suffix(entries);
                        println!(
                            "  {} -> Workshop {} [content hash: {}{}]",
                            name,
                            chosen.published_id,
                            sanitize_for_terminal(&chosen.path),
                            shared
                        );
                        continue;
                    }
                    // The name is a candidate but the bytes cannot be read,
                    // so the candidates stay unconfirmed.
                    Confirmation::Unreadable => {
                        let ids = candidates.ids().join(", ");
                        let basis = if candidates.exact_size {
                            "name and size"
                        } else {
                            "name only"
                        };
                        println!(
                            "  {} -> candidates {} [{}; the file could not be read]",
                            name, ids, basis
                        );
                        continue;
                    }
                    Confirmation::Miss => {}
                }
            }
        }

        let origin = prefix_origin
            .map(|r| r.id)
            .unwrap_or_else(|| "Unknown".to_string());
        println!("  {} -> Workshop {}", name, origin);
    }
    Ok(())
}
