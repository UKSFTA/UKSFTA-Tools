use std::fs;
use std::path::Path;

use crate::error::UksftaError;
use crate::origin::{build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id};
use crate::pbo::pbo_prefix;
use crate::steam::find_all_workshop_caches;

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
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .map(|e| e == "pbo")
                .unwrap_or(false)
            {
                let name = entry.file_name().to_string_lossy().to_string();
                let target_prefix = pbo_prefix(&entry.path());
                let candidates = index
                    .get(name.as_str())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let origin =
                    resolve_pbo_from_index(&entry.path(), candidates, target_prefix.as_deref())
                        .map(|r| r.id)
                        .unwrap_or_else(|| "Unknown".to_string());
                println!("  {} -> Workshop {}", name, origin);
            }
        }
    }
    Ok(())
}
