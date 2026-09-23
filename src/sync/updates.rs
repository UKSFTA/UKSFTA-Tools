use std::path::Path;

use crate::error::UksftaError;
use crate::lock::load_lock;
use crate::steam::load_workshop_items;

/// Compare locked timestamps against the Workshop cache.
pub fn check_updates() -> Result<(), UksftaError> {
    let lock_path = Path::new("mods.lock");
    if !lock_path.exists() {
        return Err(UksftaError::Input(
            "No mods.lock found. Run sync first.".to_string(),
        ));
    }

    let lock = load_lock(lock_path)?;

    // Load ACF metadata from all Steam libraries.
    let workshop_items = load_workshop_items();

    if workshop_items.is_empty() {
        return Err(UksftaError::Input("Workshop cache not found".to_string()));
    }

    println!("Update Check:");
    let mut updatable = 0;

    for (id, entry) in &lock.mods {
        if let Some(item) = workshop_items.get(id) {
            let lock_time: u64 = entry.updated.parse().unwrap_or(0);
            if item.time_updated > lock_time {
                println!(
                    "  {} ({}) - Workshop updated: {} > locked: {}",
                    entry.name, id, item.time_updated, lock_time
                );
                updatable += 1;
            }
        }
    }

    if updatable == 0 {
        println!("  All mods up to date");
    } else {
        println!("\n{} mods have updates available", updatable);
        println!("Run 'uksfta sync' to apply updates");
    }
    Ok(())
}
