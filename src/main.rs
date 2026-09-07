use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const STEAM_APP_ID: &str = "107410";

// --- VDF Metadata ---

#[derive(Debug, Clone, Default)]
struct WorkshopItem {
    size: u64,
    time_updated: u64,
}

fn parse_acf(content: &str) -> HashMap<String, WorkshopItem> {
    let mut items = HashMap::new();
    let mut current_id: Option<String> = None;
    let mut current_item = WorkshopItem::default();
    let mut in_installed = false;
    let mut brace_depth = 0;

    for line in content.lines() {
        let line = line.trim();

        if line.contains("\"WorkshopItemsInstalled\"") {
            in_installed = true;
            brace_depth = 0;
            continue;
        }

        if in_installed {
            if line.starts_with('{') {
                brace_depth += 1;
                continue;
            }
            if line.starts_with('}') {
                brace_depth -= 1;
                if brace_depth == 0 {
                    // Save last item if any
                    if let Some(id) = &current_id {
                        items.insert(id.clone(), current_item.clone());
                        current_id = None;
                        current_item = WorkshopItem::default();
                    }
                    in_installed = false;
                    continue;
                }
                continue;
            }

            // At depth 1, keys are published IDs
            if brace_depth == 1 {
                if let Some(val) = extract_quoted(line) {
                    if val.chars().all(|c| c.is_ascii_digit()) {
                        // Save previous item
                        if let Some(id) = &current_id {
                            items.insert(id.clone(), current_item.clone());
                        }
                        current_id = Some(val);
                        current_item = WorkshopItem::default();
                    }
                }
            }

            // At depth 2, we're inside an item's fields
            if brace_depth == 2 {
                let tokens = extract_quoted_tokens(line);
                if tokens.len() >= 2 {
                    if tokens[0] == "size" {
                        current_item.size = tokens[1].parse().unwrap_or(0);
                    } else if tokens[0] == "timeupdated" {
                        current_item.time_updated = tokens[1].parse().unwrap_or(0);
                    }
                }
            }
        }
    }

    // Save last item
    if let Some(id) = &current_id {
        items.insert(id.clone(), current_item);
    }

    items
}

fn extract_quoted(line: &str) -> Option<String> {
    let chars = line.chars();
    let mut in_quote = false;
    let mut value = String::new();

    for c in chars {
        if c == '"' {
            if in_quote {
                return Some(value);
            }
            in_quote = true;
        } else if in_quote {
            value.push(c);
        }
    }
    None
}

fn extract_quoted_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars = line.chars();
    let mut in_quote = false;
    let mut value = String::new();

    for c in chars {
        if c == '"' {
            if in_quote {
                tokens.push(std::mem::take(&mut value));
                in_quote = false;
            } else {
                in_quote = true;
            }
        } else if in_quote {
            value.push(c);
        }
    }
    tokens
}

// --- mod.cpp / meta.cpp parsing ---

#[derive(Debug, Clone, Default)]
struct ModCpp {
    name: String,
    author: String,
    publishedid: Option<String>,
}

fn parse_mod_cpp(path: &Path) -> ModCpp {
    let mut result = ModCpp::default();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = extract_quoted(line) {
                if line.starts_with("name") {
                    result.name = val;
                } else if line.starts_with("author") {
                    result.author = val;
                }
            }
        }
    }
    result
}

fn parse_meta_cpp(path: &Path) -> ModCpp {
    let mut result = ModCpp::default();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = extract_quoted(line) {
                if line.starts_with("publishedid") {
                    result.publishedid = Some(val);
                } else if line.starts_with("name") {
                    result.name = val;
                }
            }
        }
    }
    result
}

fn get_mod_metadata(mod_dir: &Path) -> ModCpp {
    let mut meta = ModCpp::default();

    // mod.cpp has presentation metadata
    let mod_cpp = mod_dir.join("mod.cpp");
    if mod_cpp.exists() {
        meta = parse_mod_cpp(&mod_cpp);
    }

    // meta.cpp has Workshop ID
    let meta_cpp = mod_dir.join("meta.cpp");
    if meta_cpp.exists() {
        let workshop_meta = parse_meta_cpp(&meta_cpp);
        if let Some(id) = workshop_meta.publishedid {
            meta.publishedid = Some(id);
        }
        if meta.name.is_empty() && !workshop_meta.name.is_empty() {
            meta.name = workshop_meta.name;
        }
    }

    meta
}

#[derive(Parser)]
#[command(name = "uksft", about = "UKSFTA modpack manager")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync mods from Workshop cache into addons/
    Sync {
        /// Show what would change without doing it
        #[arg(long)]
        dry_run: bool,
        /// Skip dependency resolution (offline mode)
        #[arg(long)]
        offline: bool,
    },
    /// Show which PBOs came from which Workshop mod
    Identify,
    /// Verify all PBOs are present
    Verify,
    /// Check for updates available in Workshop cache
    Updates,
}

// --- Mod list parsing ---

#[derive(Debug, Clone)]
struct ModEntry {
    id: String,
    name: String,
    #[allow(dead_code)]
    tag: Option<String>,
}

fn parse_mod_sources(path: &Path) -> (Vec<ModEntry>, Vec<String>) {
    let content = fs::read_to_string(path).expect("Failed to read mod_sources.txt");
    let mut mods = Vec::new();
    let mut ignored = Vec::new();
    let mut in_ignore = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            if line.to_lowercase().contains("[ignore]") || line.to_lowercase().contains("@ignore") {
                in_ignore = true;
            }
            continue;
        }
        if in_ignore {
            if let Some(id) = extract_id(line) {
                ignored.push(id);
            }
            continue;
        }
        if line.to_lowercase().contains("[ignore]") || line.to_lowercase().contains("@ignore") {
            in_ignore = true;
            continue;
        }
        if let Some(id) = extract_id(line) {
            let name = extract_tag(line).unwrap_or_else(|| format!("Mod {}", id));
            let tag = extract_tag(line);
            mods.push(ModEntry { id, name, tag });
        }
    }
    (mods, ignored)
}

fn extract_id(line: &str) -> Option<String> {
    // Match 8+ digit IDs in URLs or bare
    for word in line.split_whitespace() {
        if let Some(pos) = word.find("id=") {
            let rest = &word[pos + 3..];
            if let Some(end) = rest.find(|c: char| !c.is_ascii_digit()) {
                let id = &rest[..end];
                if id.len() >= 8 {
                    return Some(id.to_string());
                }
            } else if rest.len() >= 8 {
                return Some(rest.to_string());
            }
        }
        // Bare ID
        if word.chars().all(|c| c.is_ascii_digit()) && word.len() >= 8 {
            return Some(word.to_string());
        }
    }
    None
}

fn extract_tag(line: &str) -> Option<String> {
    if let Some(pos) = line.find('#') {
        let tag = line[pos + 1..].trim().to_string();
        if !tag.is_empty() {
            return Some(tag);
        }
    }
    None
}

// --- Steam library discovery ---

fn find_steam_library_folders() -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let mut folders = Vec::new();

    // Parse libraryfolders.vdf for all Steam libraries
    // Note: VDF is in config/ directory, not steamapps/
    let vdf_paths = [
        home.join(".steam/steam/config/libraryfolders.vdf"),
        home.join(".local/share/Steam/config/libraryfolders.vdf"),
        home.join("Steam/config/libraryfolders.vdf"),
        home.join(".steam/steam/steamapps/libraryfolders.vdf"), // fallback
        home.join(".local/share/Steam/steamapps/libraryfolders.vdf"), // fallback
    ];

    for vdf_path in &vdf_paths {
        if let Ok(content) = fs::read_to_string(vdf_path) {
            parse_libraryfolders_vdf(&content, &mut folders);
        }
    }

    // Also check common locations
    let common = [
        home.join(".steam/steam/steamapps"),
        home.join(".local/share/Steam/steamapps"),
        home.join("Steam/steamapps"),
        PathBuf::from("/ext/SteamLibrary/steamapps"),
        home.join(".steam/steamcmd/steamapps"),
    ];
    for path in &common {
        if path.exists() && !folders.contains(path) {
            folders.push(path.clone());
        }
    }

    folders
}

fn parse_libraryfolders_vdf(content: &str, folders: &mut Vec<PathBuf>) {
    for line in content.lines() {
        let line = line.trim();
        if line.contains("\"path\"") {
            if let Some(start) = line.find('"') {
                let rest = &line[start + 1..];
                if let Some(end) = rest.find('"') {
                    let path_str = &rest[..end];
                    let steamapps = PathBuf::from(path_str).join("steamapps");
                    if !folders.contains(&steamapps) {
                        folders.push(steamapps);
                    }
                }
            }
        }
    }
}

fn find_all_workshop_caches() -> Vec<PathBuf> {
    let folders = find_steam_library_folders();
    let mut caches = Vec::new();
    for folder in folders {
        let cache = folder.join("workshop/content").join(STEAM_APP_ID);
        if cache.exists() {
            caches.push(cache);
        }
    }
    caches
}

// --- Lock file ---

#[derive(Debug, Serialize, Deserialize)]
struct LockFile {
    mods: HashMap<String, ModLockEntry>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ModLockEntry {
    files: Vec<String>,
    name: String,
    dependencies: Vec<Dependency>,
    updated: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Dependency {
    id: String,
    name: String,
}

fn load_lock(path: &Path) -> LockFile {
    if path.exists() {
        let content = fs::read_to_string(path).expect("Failed to read mods.lock");
        serde_json::from_str(&content).unwrap_or(LockFile {
            mods: HashMap::new(),
        })
    } else {
        LockFile {
            mods: HashMap::new(),
        }
    }
}

fn save_lock(path: &Path, lock: &LockFile) {
    let json = serde_json::to_string_pretty(lock).expect("Failed to serialize lock");
    fs::write(path, json).expect("Failed to write mods.lock");
}

// --- PBO sync ---

fn find_pbos(dir: &Path) -> Vec<PathBuf> {
    let mut pbos = Vec::new();
    if !dir.exists() {
        return pbos;
    }
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry.expect("Failed to read directory");
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                if ext.to_string_lossy().to_lowercase() == "pbo" {
                    pbos.push(entry.path().to_path_buf());
                }
            }
        }
    }
    pbos
}

fn sync_mods(mods: &[ModEntry], ignored: &[String], dry_run: bool, _offline: bool) {
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
    let mut missing_from_cache = Vec::new();

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
                missing_from_cache.push(entry.id.clone());
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
            missing_from_cache.push(entry.id.clone());
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

        planned.push((
            ModLockEntry {
                files,
                name: display_name,
                dependencies: Vec::new(),
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
        for id in &missing_from_cache {
            println!("  [MISSING] Mod {} not found in Workshop cache", id);
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
        mods: HashMap::new(),
    };

    for (entry, id, status, sources) in &planned {
        if status == "unchanged" {
            // Keep the existing lock entry as-is
            new_lock.mods.insert(id.clone(), lock.mods[id].clone());
            continue;
        }

        fs::create_dir_all(addons_dir).expect("Failed to create addons directory");
        // files[i] is the dest path, sources[i] is the real source path
        for (dest, src) in entry.files.iter().zip(sources.iter()) {
            fs::copy(src, dest).expect("Failed to copy PBO");
        }
        new_lock.mods.insert(id.clone(), entry.clone());
        println!(
            "{}: {} ({})",
            if status == "added" {
                "Added"
            } else {
                "Updated"
            },
            entry.name,
            id
        );
    }

    // Remove PBOs for mods no longer in the list
    for (id, name, files) in &removed {
        for file in files {
            if Path::new(file).exists() {
                fs::remove_file(file).expect("Failed to remove PBO");
            }
        }
        println!("Removed: {} ({})", name, id);
    }

    save_lock(lock_path, &new_lock);

    for id in &missing_from_cache {
        eprintln!("Warning: Mod {} not found in Workshop cache", id);
    }

    println!(
        "\nSummary: {} added, {} updated, {} unchanged, {} removed",
        added,
        updated,
        unchanged,
        removed.len()
    );
}

fn identify() {
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

    // Build map of PBO -> workshop ID from all caches
    let mut pbo_map: HashMap<String, String> = HashMap::new();
    for cache in &caches {
        if let Ok(entries) = fs::read_dir(cache) {
            for entry in entries.flatten() {
                let mod_path = entry.path();
                if mod_path.is_dir() {
                    let id = mod_path.file_name().unwrap().to_string_lossy().to_string();
                    for pbo in find_pbos(&mod_path) {
                        if let Some(name) = pbo.file_name() {
                            pbo_map.insert(name.to_string_lossy().to_string(), id.clone());
                        }
                    }
                }
            }
        }
    }

    println!("PBO Origins:");
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .map(|e| e == "pbo")
                .unwrap_or(false)
            {
                let name = entry.file_name().to_string_lossy().to_string();
                let origin = pbo_map.get(&name).map(|s| s.as_str()).unwrap_or("Unknown");
                println!("  {} -> Workshop {}", name, origin);
            }
        }
    }
}

fn verify() {
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

fn check_updates() {
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
        println!("Run 'uksft sync' to apply updates");
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sync { dry_run, offline } => {
            let (mods, ignored) = parse_mod_sources(Path::new("mod_sources.txt"));
            if mods.is_empty() {
                eprintln!("No mods found in mod_sources.txt");
                return;
            }
            println!("Found {} mods, {} ignored", mods.len(), ignored.len());
            sync_mods(&mods, &ignored, dry_run, offline);
        }
        Commands::Identify => identify(),
        Commands::Verify => verify(),
        Commands::Updates => check_updates(),
    }
}
