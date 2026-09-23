use std::path::Path;

use crate::error::UksftaError;
use crate::lock::load_lock;

/// Confirm every PBO recorded in the lock is present.
pub fn verify() -> Result<(), UksftaError> {
    let lock_path = Path::new("mods.lock");
    if !lock_path.exists() {
        return Err(UksftaError::Input(
            "No mods.lock found. Run sync first.".to_string(),
        ));
    }

    let lock = load_lock(lock_path)?;
    let mut missing = 0;

    for (id, entry) in &lock.mods {
        for file in &entry.files {
            if !Path::new(file).exists() {
                eprintln!("Missing: {} ({}) -> {}", entry.name, id, file);
                missing += 1;
            }
        }
    }

    if missing > 0 {
        return Err(UksftaError::Check(format!("{missing} PBOs missing")));
    }
    println!(
        "All {} mods verified ({} PBOs)",
        lock.mods.len(),
        lock.mods.values().map(|m| m.files.len()).sum::<usize>()
    );
    Ok(())
}
