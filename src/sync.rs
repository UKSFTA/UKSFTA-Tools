use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::lock::{default_lock_version, load_lock, save_lock, Dependency, LockFile, ModLockEntry};
use crate::modlist::{
    generate_modlist, parse_mod_sources, print_dep_tree, resolve_transitive_deps, ModEntry,
};
use crate::origin::{build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id};
use crate::pbo::{get_mod_metadata, pbo_prefix};
use crate::steam::{find_all_workshop_caches, find_steam_library_folders, parse_acf, STEAM_APP_ID};
use crate::util::find_pbos;

pub fn sync_mods(
    mods: &[ModEntry],
    ignored: &[String],
    dry_run: bool,
    _offline: bool,
    modlist: bool,
    modlist_path: &Path,
    resolve_deps: bool,
) {
    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        eprintln!("Workshop cache not found. Is Steam installed?");
        return;
    }
    let addons_dir = Path::new("addons");
    let lock_path = Path::new("mods.lock");

    let lock = load_lock(lock_path);

    // Load VDF metadata for timestamps from all caches
    let mut workshop_items = HashMap::new();
    for cache in &caches {
        // cache = .../workshop/content/107410
        // ACF is at .../workshop/appworkshop_107410.acf
        if let Some(workshop_dir) = cache.parent().and_then(|p| p.parent()) {
            let acf = workshop_dir.join(format!("appworkshop_{}.acf", STEAM_APP_ID));
            if acf.exists() {
                if let Ok(content) = fs::read_to_string(&acf) {
                    workshop_items.extend(parse_acf(&content));
                }
            }
        }
    }

    // Classify each requested mod: added / updated / unchanged / missing from cache
    let mut planned: Vec<(ModLockEntry, String, String, Vec<PathBuf>)> = Vec::new(); // (entry, id, status, source PBO paths)
    let mut missing_from_cache = Vec::new(); // (id, name)

    for entry in mods {
        // Find the mod in any cache
        let mut mod_path = None;
        for cache in &caches {
            let path = cache.join(&entry.id);
            if path.exists() {
                mod_path = Some(path);
                break;
            }
        }

        let mod_path = match mod_path {
            Some(p) => p,
            None => {
                missing_from_cache.push((entry.id.clone(), entry.name.clone()));
                continue;
            }
        };

        // Get metadata from mod.cpp/meta.cpp if available
        let mod_meta = get_mod_metadata(&mod_path);
        let display_name = if mod_meta.name.is_empty() {
            entry.name.clone()
        } else {
            mod_meta.name
        };

        let pbos = find_pbos(&mod_path);
        if pbos.is_empty() {
            missing_from_cache.push((entry.id.clone(), entry.name.clone()));
            continue;
        }

        // Timestamp from ACF (Workshop's last update)
        let updated = workshop_items
            .get(&entry.id)
            .map(|item| item.time_updated.to_string())
            .unwrap_or_else(|| "0".to_string());

        let files: Vec<String> = pbos
            .iter()
            .map(|p| {
                addons_dir
                    .join(p.file_name().unwrap())
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        // Classify against lock
        let status = match lock.mods.get(&entry.id) {
            None => "added",
            Some(locked) => {
                let files_exist = locked.files.iter().all(|f| Path::new(f).exists());
                if locked.updated != updated || !files_exist {
                    "updated"
                } else {
                    "unchanged"
                }
            }
        };

        // Resolve declared dependency names from the mods list where possible
        let dependencies: Vec<Dependency> = entry
            .dependencies
            .iter()
            .map(|dep_id| Dependency {
                id: dep_id.clone(),
                name: mods
                    .iter()
                    .find(|m| &m.id == dep_id)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
            })
            .collect();

        planned.push((
            ModLockEntry {
                files,
                name: display_name,
                tags: entry.tags.clone(),
                dependencies,
                updated,
            },
            entry.id.clone(),
            status.to_string(),
            pbos,
        ));
    }

    // Removed: in lock but not in current mod list (and not ignored)
    let mut removed: Vec<(String, String, Vec<String>)> = Vec::new(); // (id, name, files)
    for (id, old_entry) in &lock.mods {
        if !mods.iter().any(|m| &m.id == id) && !ignored.contains(id) {
            removed.push((id.clone(), old_entry.name.clone(), old_entry.files.clone()));
        }
    }

    let added = planned.iter().filter(|(_, _, s, _)| s == "added").count();
    let updated = planned.iter().filter(|(_, _, s, _)| s == "updated").count();
    let unchanged = planned
        .iter()
        .filter(|(_, _, s, _)| s == "unchanged")
        .count();

    // Print the diff preview
    if dry_run {
        println!("--- Diff preview (dry-run) ---");
        for (entry, id, status, _sources) in &planned {
            match status.as_str() {
                "added" => {
                    println!("  [ADD]     {} ({})", entry.name, id);
                    for f in &entry.files {
                        println!(
                            "    + {}",
                            Path::new(f).file_name().unwrap().to_string_lossy()
                        );
                    }
                }
                "updated" => {
                    println!("  [UPDATE]  {} ({})", entry.name, id);
                    for f in &entry.files {
                        println!(
                            "    ~ {}",
                            Path::new(f).file_name().unwrap().to_string_lossy()
                        );
                    }
                }
                _ => println!(
                    "  [OK]      {} ({}) — {} PBOs present",
                    entry.name,
                    id,
                    entry.files.len()
                ),
            }
        }
        for (id, name, files) in &removed {
            println!("  [REMOVE]  {} ({})", name, id);
            for f in files {
                println!(
                    "    - {}",
                    Path::new(f).file_name().unwrap().to_string_lossy()
                );
            }
        }
        for (id, name) in &missing_from_cache {
            println!("  [MISSING] {} ({}) not found in Workshop cache", name, id);
            println!(
                "            https://steamcommunity.com/sharedfiles/filedetails/?id={}",
                id
            );
        }
        if modlist && !missing_from_cache.is_empty() {
            println!("\nModlist would be written to {}", modlist_path.display());
        }
        println!(
            "\nSummary: {} added, {} updated, {} unchanged, {} removed",
            added,
            updated,
            unchanged,
            removed.len()
        );
        return;
    }

    // Apply changes
    let mut new_lock = LockFile {
        version: default_lock_version(),
        mods: HashMap::new(),
    };

    let work_items: Vec<&(ModLockEntry, String, String, Vec<PathBuf>)> = planned
        .iter()
        .filter(|(_, _, status, _)| status != "unchanged")
        .collect();

    let bar = indicatif::ProgressBar::new(work_items.len() as u64);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} mods {msg}",
        )
        .unwrap()
        .progress_chars("##-"),
    );

    for (entry, id, _status, sources) in work_items {
        bar.set_message(format!("{} ({})", entry.name, id));

        fs::create_dir_all(addons_dir).expect("Failed to create addons directory");
        // files[i] is the dest path, sources[i] is the real source path
        for (dest, src) in entry.files.iter().zip(sources.iter()) {
            fs::copy(src, dest).expect("Failed to copy PBO");
        }
        new_lock.mods.insert(id.clone(), entry.clone());
        bar.inc(1);
    }
    bar.finish_and_clear();

    for (_entry, id, status, _) in &planned {
        if status == "unchanged" {
            // Keep the existing lock entry as-is
            new_lock.mods.insert(id.clone(), lock.mods[id].clone());
        }
    }

    // Print the applied changes summary
    println!(
        "\nSynced: {} added, {} updated, {} unchanged, {} removed",
        added,
        updated,
        unchanged,
        removed.len()
    );

    // Remove PBOs for mods no longer in the list
    for (id, name, files) in &removed {
        for file in files {
            if Path::new(file).exists() {
                fs::remove_file(file).expect("Failed to remove PBO");
            }
        }
        println!("Removed: {} ({})", name, id);
    }

    // Preserve lock entries for mods still in the list but missing from
    // the Workshop cache. They are wanted but not currently syncable, so
    // their previous file records must not be dropped.
    for (id, _) in &missing_from_cache {
        if let Some(entry) = lock.mods.get(id) {
            new_lock.mods.insert(id.clone(), entry.clone());
        }
    }

    // Only write the lock when its content actually changed. This avoids
    // touching the file (and dirtying the working tree) when nothing was
    // synced or removed.
    let changed = new_lock.mods != lock.mods;
    if changed {
        save_lock(lock_path, &new_lock);
    }

    // Resolve dependencies once (only when requested) so warnings and
    // modlist both benefit without double-fetching Workshop pages.
    let (mods_for_list, deps_by_mod) = if resolve_deps {
        let known_ids: std::collections::HashSet<String> =
            mods.iter().map(|m| m.id.clone()).collect();
        let cached_ids: std::collections::HashSet<String> = caches
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
        resolve_transitive_deps(&missing_from_cache, &known_ids, &cached_ids)
    } else {
        (missing_from_cache.clone(), HashMap::new())
    };

    for (id, name) in &missing_from_cache {
        eprintln!("Warning: {} ({}) not found in Workshop cache", name, id);
        if resolve_deps {
            let mut visited = HashSet::new();
            visited.insert(id.clone());
            print_dep_tree(id, &deps_by_mod, "         ", &mut visited);
        }
        eprintln!(
            "         https://steamcommunity.com/sharedfiles/filedetails/?id={}",
            id
        );
    }

    if !missing_from_cache.is_empty() {
        println!(
            "\nSubscribe to the {} missing mods in Steam:",
            missing_from_cache.len()
        );
        for (id, _) in &missing_from_cache {
            println!("steam://url/CommunityFilePage/{}", id);
        }

        if modlist {
            generate_modlist(&mods_for_list, modlist_path);
        }
    } else if modlist {
        println!("No missing mods — modlist not generated.");
    }

    println!(
        "\nSummary: {} added, {} updated, {} unchanged, {} removed",
        added,
        updated,
        unchanged,
        removed.len()
    );
}

pub fn identify() {
    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        eprintln!("Workshop cache not found");
        return;
    }

    let addons_dir = Path::new("addons");
    if !addons_dir.exists() {
        eprintln!("No addons/ directory");
        return;
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
}

pub fn verify() {
    let lock_path = Path::new("mods.lock");
    if !lock_path.exists() {
        eprintln!("No mods.lock found. Run sync first.");
        return;
    }

    let lock = load_lock(lock_path);
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
        eprintln!("\n{} PBOs missing", missing);
        std::process::exit(1);
    } else {
        println!(
            "All {} mods verified ({} PBOs)",
            lock.mods.len(),
            lock.mods.values().map(|m| m.files.len()).sum::<usize>()
        );
    }
}

pub fn audit(missing_only: bool) {
    let (mods, _ignored) = parse_mod_sources(Path::new("mod_sources.txt"));
    if mods.is_empty() {
        eprintln!("No mods found in mod_sources.txt");
        return;
    }

    let mod_count = mods.len();

    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        eprintln!("Workshop cache not found. Is Steam installed?");
        return;
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
        let mut mod_path = None;
        for cache in &caches {
            let path = cache.join(&entry.id);
            if path.exists() {
                mod_path = Some(path);
                break;
            }
        }

        let mod_path = match mod_path {
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
        std::process::exit(1);
    }
}

pub fn check_updates() {
    let lock_path = Path::new("mods.lock");
    if !lock_path.exists() {
        eprintln!("No mods.lock found. Run sync first.");
        return;
    }

    let lock = load_lock(lock_path);

    // Load ACF from all Steam libraries
    let mut workshop_items = HashMap::new();
    let folders = find_steam_library_folders();
    for folder in &folders {
        let acf = folder
            .join("workshop")
            .join(format!("appworkshop_{}.acf", STEAM_APP_ID));
        if acf.exists() {
            if let Ok(content) = fs::read_to_string(&acf) {
                workshop_items.extend(parse_acf(&content));
            }
        }
    }

    if workshop_items.is_empty() {
        eprintln!("Workshop cache not found");
        return;
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
}
