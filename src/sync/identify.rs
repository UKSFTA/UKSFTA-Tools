use std::path::Path;

use crate::error::UksftaError;
use crate::origin::{build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id};
use crate::pbo::pbo_prefix;
use crate::steam::find_all_workshop_caches;
use crate::util::find_pbos;

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
    for pbo in find_pbos(addons_dir) {
        let Some(name) = pbo.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let target_prefix = pbo_prefix(&pbo);
        let candidates = index.candidates(&name);
        let origin = resolve_pbo_from_index(&pbo, candidates, target_prefix.as_deref())
            .map(|r| r.id)
            .unwrap_or_else(|| "Unknown".to_string());
        println!("  {} -> Workshop {}", name, origin);
    }
    Ok(())
}
