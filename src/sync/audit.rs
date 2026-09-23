use std::collections::HashMap;
use std::path::Path;

use crate::error::UksftaError;
use crate::modlist::parse_mod_sources;
use crate::steam::{find_all_workshop_caches, find_mod_in_caches};
use crate::util::find_pbos;

/// Audit each mod's PBOs against addons/ (present/missing per PBO).
pub fn audit(missing_only: bool) -> Result<(), UksftaError> {
    let (mods, _ignored) = parse_mod_sources(Path::new("mod_sources.txt"))?;
    if mods.is_empty() {
        return Err(UksftaError::Input(
            "No mods found in mod_sources.txt".to_string(),
        ));
    }

    let mod_count = mods.len();

    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        return Err(UksftaError::Input(
            "Workshop cache not found. Is Steam installed?".to_string(),
        ));
    }

    let addons_dir = Path::new("addons");

    // Build a set of every PBO filename currently in addons/ (recursive)
    let mut present_pbos: HashMap<String, ()> = HashMap::new();
    for pbo in find_pbos(addons_dir) {
        if let Some(name) = pbo.file_name().map(|n| n.to_string_lossy().to_string()) {
            present_pbos.insert(name, ());
        }
    }

    // Collect every expected PBO name across all mods, for orphan detection
    let mut all_expected: HashMap<String, String> = HashMap::new(); // pbo name -> mod name
    let mut not_in_cache = 0;
    let mut total_missing = 0;
    let mut total_expected = 0;
    let mut missing_list: Vec<String> = Vec::new();

    println!("--- Mod audit ---");

    for entry in &mods {
        // Find this mod in any cache
        let mod_path = match find_mod_in_caches(&caches, &entry.id) {
            Some(p) => p,
            None => {
                println!(
                    "  [NOT IN CACHE] {} ({}) — cannot enumerate PBOs",
                    entry.name, entry.id
                );
                not_in_cache += 1;
                continue;
            }
        };

        // Expected PBOs from the cache, by filename
        let expected = find_pbos(&mod_path);
        if expected.is_empty() {
            println!(
                "  [EMPTY]   {} ({}) — no PBOs in cache",
                entry.name, entry.id
            );
            continue;
        }

        let expected_names: Vec<String> = expected
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .collect();
        total_expected += expected_names.len();

        let present_count = expected_names
            .iter()
            .filter(|n| present_pbos.contains_key(*n))
            .count();
        let missing: Vec<&String> = expected_names
            .iter()
            .filter(|n| !present_pbos.contains_key(*n))
            .collect();

        for name in &expected_names {
            all_expected
                .entry(name.clone())
                .or_insert_with(|| entry.name.clone());
        }

        let status = if missing.is_empty() {
            "OK"
        } else {
            "INCOMPLETE"
        };
        if !missing_only || !missing.is_empty() {
            println!(
                "  [{}] {} ({}) — {}/{} PBOs",
                status,
                entry.name,
                entry.id,
                present_count,
                expected_names.len()
            );
        }

        if !missing_only {
            for name in &expected_names {
                if present_pbos.contains_key(name) {
                    println!("      present: {}", name);
                } else {
                    println!("      MISSING: {}", name);
                }
            }
        } else {
            for name in &missing {
                println!("      MISSING: {}", name);
            }
        }

        for name in &missing {
            missing_list.push(format!("{} -> {}", name, entry.name));
        }
        total_missing += missing.len();
    }

    // Orphan detection: PBOs in addons/ that belong to no expected mod
    let orphans: Vec<&String> = present_pbos
        .keys()
        .filter(|name| !all_expected.contains_key(*name))
        .collect();
    if !orphans.is_empty() {
        println!("\n  [ORPHANS] PBOs in addons/ not from any listed mod:");
        for name in &orphans {
            println!("      orphan: {}", name);
        }
    }

    let pct = total_expected
        .checked_sub(total_missing)
        .and_then(|n| n.checked_mul(100))
        .and_then(|n| n.checked_div(total_expected))
        .unwrap_or(100);

    println!(
        "\nSummary: {}/{} PBOs present ({}%) across {} mods; {} not in cache",
        total_expected - total_missing,
        total_expected,
        pct,
        mod_count,
        not_in_cache
    );

    if !missing_list.is_empty() {
        println!("\nMissing PBOs:");
        for m in &missing_list {
            println!("  {}", m);
        }
        return Err(UksftaError::Check(format!(
            "{} PBOs missing",
            missing_list.len()
        )));
    }
    Ok(())
}
