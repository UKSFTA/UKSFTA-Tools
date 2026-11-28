use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::error::UksftaError;
use crate::lock::{load_lock, LockFile, ModLockEntry};
use crate::modlist::{parse_mod_sources, parse_modlist_html, ModEntry};
use crate::steam::{fetch_workshop_sizes, load_workshop_items, WorkshopItem};
use crate::util::{
    find_pbos, human_size, paint, stdout_is_tty, C_BOLD, C_DIM, C_GREEN, C_RED, C_YELLOW,
};

/// Print each Steam mod in an Arma 3 launcher modlist HTML file with its
/// download size from the local Workshop cache, then the total. With
/// `online`, sizes for mods not installed locally come from the keyless
/// GetPublishedFileDetails API.
pub fn modlist_size(modlist_file: &Path, online: bool) -> Result<(), UksftaError> {
    let content = fs::read_to_string(modlist_file).map_err(|e| {
        UksftaError::Input(format!("cannot read {}: {}", modlist_file.display(), e))
    })?;

    let (steam_mods, local_mods) = parse_modlist_html(&content);
    if steam_mods.is_empty() && local_mods.is_empty() {
        return Err(UksftaError::Input(format!(
            "No mods found in {}. Is it an Arma 3 launcher modlist?",
            modlist_file.display()
        )));
    }

    let mut items = load_workshop_items();
    if online {
        let missing = missing_size_ids(&steam_mods, &items);
        if !missing.is_empty() {
            println!(
                "Fetching {} mod size(s) from the Steam Workshop API...",
                missing.len()
            );
            for (id, size) in fetch_workshop_sizes(&missing) {
                // Only the size is used from here on; the timestamp is unused.
                items.insert(
                    id,
                    WorkshopItem {
                        latest_manifest: String::new(),
                        size,
                        time_updated: 0,
                        manifest: String::new(),
                    },
                );
            }
        }
    }
    let sizes = lookup_sizes(&steam_mods, &items);
    let use_colour = stdout_is_tty();
    let total: u64 = sizes.iter().filter_map(|(_, _, size)| *size).sum();

    println!("Modlist size:");
    println!("  {} mod(s) in preset", sizes.len());
    if use_colour {
        println!(
            "  {}",
            paint(
                use_colour,
                C_DIM,
                "colour: green <5%, yellow <20%, red >=20% of total"
            )
        );
    }

    let mut unknown = 0;
    for (id, name, size) in &sizes {
        match size {
            Some(bytes) => {
                let share = if total > 0 {
                    *bytes as f64 / total as f64
                } else {
                    0.0
                };
                let cell = format!("{:>10}", human_size(*bytes));
                println!(
                    "  {}  {} ({})",
                    paint(use_colour, size_share_colour(share), &cell),
                    name,
                    id
                );
            }
            None => {
                unknown += 1;
                let cell = format!("{:>10}", "unknown");
                println!("  {}  {} ({})", paint(use_colour, C_DIM, &cell), name, id);
            }
        }
    }

    println!();
    println!("{}", paint(use_colour, C_BOLD, "Summary:"));
    println!(
        "  Total: {} ({} of {} mods)",
        paint(use_colour, C_BOLD, &human_size(total)),
        sizes.len() - unknown,
        sizes.len()
    );
    if unknown > 0 {
        let hint = if online {
            String::new()
        } else {
            paint(
                use_colour,
                C_DIM,
                " Run with --online to fetch the missing sizes.",
            )
        };
        println!("  {} mod(s) not in the Workshop cache.{}", unknown, hint);
    }
    if !local_mods.is_empty() {
        println!(
            "  {} local mod(s) with no Workshop size: {}",
            local_mods.len(),
            local_mods.join(", ")
        );
    }
    Ok(())
}

/// Colour for a mod's share of the total size: green under 5%, yellow
/// under 20%, red at or above 20%.
pub fn size_share_colour(share: f64) -> &'static str {
    if share < 0.05 {
        C_GREEN
    } else if share < 0.20 {
        C_YELLOW
    } else {
        C_RED
    }
}

/// Pair each mod with its Workshop cache size. The size is None when the
/// mod is not installed locally, so the total counts only known sizes.
pub fn lookup_sizes(
    mods: &[(String, String)],
    items: &HashMap<String, WorkshopItem>,
) -> Vec<(String, String, Option<u64>)> {
    mods.iter()
        .map(|(id, name)| (id.clone(), name.clone(), items.get(id).map(|i| i.size)))
        .collect()
}

/// Deduplicated ids from `mods` whose size is not known locally. A size of
/// zero counts as unknown, since the ACF omits it for uninstalled items.
pub fn missing_size_ids(
    mods: &[(String, String)],
    items: &HashMap<String, WorkshopItem>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for (id, _) in mods {
        let known = items.get(id).map(|i| i.size > 0).unwrap_or(false);
        if !known && seen.insert(id.clone()) {
            ids.push(id.clone());
        }
    }
    ids
}

// --- Installed pack size ---

/// One mod's installed size in `addons/`.
pub struct ModSize {
    pub id: String,
    pub name: String,
    pub pbo_count: usize,
    pub bytes: u64,
    pub missing: usize,
}

/// Installed pack totals, derived from `mods.lock` and the files on disk.
pub struct PackSize {
    pub mods: Vec<ModSize>,
    pub tracked_bytes: u64,
    pub tracked_files: usize,
    pub untracked_count: usize,
    pub untracked_bytes: u64,
    pub untracked_keys: Vec<String>,
    pub missing_files: usize,
    pub missing: Vec<String>,
}

/// Order for the installed pack report.
#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum SortKey {
    #[default]
    Size,
    Name,
}

/// Normalise a lock path to the form `walkdir` reports from the relative
/// directory: forward slashes, no `./` prefix, lowercase.
fn path_key(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    slashed
        .strip_prefix("./")
        .unwrap_or(&slashed)
        .to_lowercase()
}

/// Sum the installed size of each mod in the lock. `disk` maps a normalised
/// path key to its file size. A file counts once across the pack, so
/// `tracked_bytes` is not the sum of the per-mod bytes.
pub fn pack_size(lock: &LockFile, disk: &HashMap<String, u64>) -> PackSize {
    let mut mods = Vec::with_capacity(lock.mods.len());
    let mut tracked_keys: HashSet<String> = HashSet::new();
    let mut tracked_bytes = 0u64;
    let mut missing_files = 0usize;
    let mut missing: BTreeSet<String> = BTreeSet::new();

    for (id, entry) in &lock.mods {
        let mut seen: HashSet<String> = HashSet::new();
        let mut bytes = 0u64;
        let mut pbo_count = 0usize;
        let mut missing_here = 0usize;

        for file in &entry.files {
            let key = path_key(file);
            if !seen.insert(key.clone()) {
                continue;
            }
            match disk.get(&key) {
                Some(size) => {
                    bytes = bytes.saturating_add(*size);
                    pbo_count += 1;
                    if tracked_keys.insert(key.clone()) {
                        tracked_bytes = tracked_bytes.saturating_add(*size);
                    }
                }
                None => {
                    missing_here += 1;
                    missing.insert(file.clone());
                }
            }
        }

        missing_files += missing_here;
        mods.push(ModSize {
            id: id.clone(),
            name: if entry.name.is_empty() {
                format!("Mod {id}")
            } else {
                entry.name.clone()
            },
            pbo_count,
            bytes,
            missing: missing_here,
        });
    }

    let mut untracked: Vec<(&String, &u64)> = disk
        .iter()
        .filter(|(key, _)| !tracked_keys.contains(*key))
        .collect();
    untracked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));

    PackSize {
        mods,
        tracked_bytes,
        tracked_files: tracked_keys.len(),
        untracked_count: untracked.len(),
        untracked_bytes: untracked.iter().map(|(_, size)| **size).sum(),
        untracked_keys: untracked.iter().map(|(key, _)| (*key).clone()).collect(),
        missing_files,
        missing: missing.into_iter().collect(),
    }
}

/// Sort the report by size (largest first) or by name (case-insensitive).
pub fn sort_mod_sizes(mods: &mut [ModSize], sort: SortKey) {
    match sort {
        SortKey::Size => mods.sort_by(|a, b| {
            b.bytes
                .cmp(&a.bytes)
                .then_with(|| a.name.cmp(&b.name))
                .then_with(|| a.id.cmp(&b.id))
        }),
        SortKey::Name => mods.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        }),
    }
}

/// PBOs of one mod that are present on disk, with the file name and size,
/// largest first.
fn mod_pbos(entry: &ModLockEntry, disk: &HashMap<String, u64>) -> Vec<(String, u64)> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut rows = Vec::new();
    for file in &entry.files {
        let key = path_key(file);
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some(size) = disk.get(&key) {
            let slashed = file.replace('\\', "/");
            let name = slashed.rsplit('/').next().unwrap_or(&slashed).to_string();
            rows.push((name, *size));
        }
    }
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows
}

/// Print the installed size of each mod in `mods.lock` against `addons/`,
/// the PBOs no lock entry tracks, and any locked PBOs that are absent.
pub fn installed_size(show_pbos: bool, sort: SortKey) -> Result<(), UksftaError> {
    let addons = Path::new("addons");
    if !addons.is_dir() {
        return Err(UksftaError::Input("No addons/ directory".to_string()));
    }

    let lock = load_lock(Path::new("mods.lock"))?;

    let mut disk: HashMap<String, u64> = HashMap::new();
    let mut display: HashMap<String, String> = HashMap::new();
    for pbo in find_pbos(addons) {
        let key = path_key(&pbo.to_string_lossy());
        match fs::metadata(&pbo) {
            Ok(meta) => {
                if let Some(name) = pbo.file_name() {
                    display.insert(key.clone(), name.to_string_lossy().to_string());
                }
                disk.insert(key, meta.len());
            }
            Err(e) => eprintln!("Warning: cannot read {}: {}", pbo.display(), e),
        }
    }

    let mut pack = pack_size(&lock, &disk);
    sort_mod_sizes(&mut pack.mods, sort);
    let use_colour = stdout_is_tty();

    println!("Installed pack size:");
    println!(
        "  {} PBO(s), {} mod(s), {}",
        pack.tracked_files,
        pack.mods.len(),
        human_size(pack.tracked_bytes)
    );
    if !pack.mods.is_empty() {
        println!();
    }

    for m in &pack.mods {
        let share = if pack.tracked_bytes > 0 {
            m.bytes as f64 / pack.tracked_bytes as f64
        } else {
            0.0
        };
        let cell = format!("{:>10}", human_size(m.bytes));
        println!(
            "  {}  {:>5.1}%  {} ({})  {} PBO(s)",
            paint(use_colour, size_share_colour(share), &cell),
            share * 100.0,
            m.name,
            m.id,
            m.pbo_count
        );
        if show_pbos {
            if let Some(entry) = lock.mods.get(&m.id) {
                for (name, size) in mod_pbos(entry, &disk) {
                    println!("    {:>10}  {}", human_size(size), name);
                }
            }
        }
    }

    println!();
    if pack.missing_files > 0 {
        println!(
            "  Missing:   {} locked PBO(s) not found in addons/",
            pack.missing_files
        );
        if show_pbos {
            for path in &pack.missing {
                let name = path.rsplit(['/', '\\']).next().unwrap_or(path.as_str());
                println!("    {name}");
            }
        }
    }
    println!(
        "  Untracked: {} PBO(s), {}",
        pack.untracked_count,
        human_size(pack.untracked_bytes)
    );
    if show_pbos {
        for key in &pack.untracked_keys {
            if let Some(size) = disk.get(key) {
                let name = display.get(key).map(String::as_str).unwrap_or(key.as_str());
                println!("    {:>10}  {}", human_size(*size), name);
            }
        }
    }

    let sources = Path::new("mod_sources.txt");
    if sources.exists() {
        match parse_mod_sources(sources) {
            Ok((mods, _ignored)) => {
                let not_installed = sources_not_in_lock(&lock, &mods);
                if !not_installed.is_empty() {
                    println!(
                        "  Not installed: {} enabled mod(s) in mod_sources.txt",
                        not_installed.len()
                    );
                    for (name, id) in &not_installed {
                        println!("    {name} ({id})");
                    }
                }
            }
            Err(e) => eprintln!("Warning: {e}"),
        }
    }
    Ok(())
}

/// Enabled `mod_sources.txt` mods that no `mods.lock` entry covers, as
/// `(name, id)` pairs sorted by name and then id.
fn sources_not_in_lock(lock: &LockFile, mods: &[ModEntry]) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = mods
        .iter()
        .filter(|m| !lock.mods.contains_key(&m.id))
        .map(|m| (m.name.clone(), m.id.clone()))
        .collect();
    rows.sort_by(|a, b| {
        a.0.to_lowercase()
            .cmp(&b.0.to_lowercase())
            .then_with(|| a.1.cmp(&b.1))
    });
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(files: &[&str], name: &str) -> ModLockEntry {
        ModLockEntry {
            files: files.iter().map(|f| f.to_string()).collect(),
            name: name.to_string(),
            tags: Vec::new(),
            dependencies: Vec::new(),
            updated: String::new(),
        }
    }

    fn lock_of(items: &[(&str, ModLockEntry)]) -> LockFile {
        LockFile {
            version: 1,
            mods: items
                .iter()
                .map(|(id, entry)| ((*id).to_string(), entry.clone()))
                .collect(),
        }
    }

    fn disk_of(items: &[(&str, u64)]) -> HashMap<String, u64> {
        items
            .iter()
            .map(|(path, size)| ((*path).to_string(), *size))
            .collect()
    }

    fn mod_entry(id: &str, name: &str) -> ModEntry {
        ModEntry {
            id: id.to_string(),
            name: name.to_string(),
            tags: Vec::new(),
            role: "include".to_string(),
            enabled: true,
            dependencies: Vec::new(),
        }
    }

    fn by_id<'a>(pack: &'a PackSize, id: &str) -> &'a ModSize {
        pack.mods
            .iter()
            .find(|m| m.id == id)
            .unwrap_or_else(|| panic!("mod {id} not in report"))
    }

    #[test]
    fn pack_size_sums_two_files_for_one_mod() {
        let lock = lock_of(&[("111", entry(&["addons/a.pbo", "addons/b.pbo"], "Mod A"))]);
        let disk = disk_of(&[("addons/a.pbo", 100), ("addons/b.pbo", 200)]);

        let pack = pack_size(&lock, &disk);

        let m = by_id(&pack, "111");
        assert_eq!(m.pbo_count, 2);
        assert_eq!(m.bytes, 300);
        assert_eq!(m.missing, 0);
        assert_eq!(pack.tracked_bytes, 300);
        assert_eq!(pack.tracked_files, 2);
        assert_eq!(pack.untracked_count, 0);
        assert_eq!(pack.missing_files, 0);
    }

    #[test]
    fn pack_size_counts_untracked_file_separately() {
        let lock = lock_of(&[("111", entry(&["addons/a.pbo"], "Mod A"))]);
        let disk = disk_of(&[("addons/a.pbo", 100), ("addons/z.pbo", 50)]);

        let pack = pack_size(&lock, &disk);

        assert_eq!(pack.tracked_bytes, 100);
        assert_eq!(pack.tracked_files, 1);
        assert_eq!(pack.untracked_count, 1);
        assert_eq!(pack.untracked_bytes, 50);
    }

    #[test]
    fn pack_size_counts_missing_locked_file_without_bytes() {
        let lock = lock_of(&[("111", entry(&["addons/a.pbo", "addons/c.pbo"], "Mod A"))]);
        let disk = disk_of(&[("addons/a.pbo", 100)]);

        let pack = pack_size(&lock, &disk);

        let m = by_id(&pack, "111");
        assert_eq!(m.pbo_count, 1);
        assert_eq!(m.bytes, 100);
        assert_eq!(m.missing, 1);
        assert_eq!(pack.missing_files, 1);
        assert_eq!(pack.tracked_bytes, 100);
    }

    #[test]
    fn pack_size_counts_a_duplicate_path_once() {
        let lock = lock_of(&[
            ("111", entry(&["addons/a.pbo", "addons/a.pbo"], "Mod A")),
            ("222", entry(&["addons/a.pbo"], "Mod B")),
        ]);
        let disk = disk_of(&[("addons/a.pbo", 100)]);

        let pack = pack_size(&lock, &disk);

        assert_eq!(by_id(&pack, "111").pbo_count, 1);
        assert_eq!(by_id(&pack, "111").bytes, 100);
        assert_eq!(by_id(&pack, "222").bytes, 100);
        assert_eq!(pack.tracked_bytes, 100);
        assert_eq!(pack.tracked_files, 1);
    }

    #[test]
    fn pack_size_matches_normalised_lock_path() {
        let lock = lock_of(&[("111", entry(&["./addons\\A.pbo", "ADDONS/B.PBO"], "Mod A"))]);
        let disk = disk_of(&[("addons/a.pbo", 10), ("addons/b.pbo", 20)]);

        let pack = pack_size(&lock, &disk);

        let m = by_id(&pack, "111");
        assert_eq!(m.pbo_count, 2);
        assert_eq!(m.bytes, 30);
        assert_eq!(pack.untracked_count, 0);
    }

    #[test]
    fn pack_size_falls_back_to_mod_id_name() {
        let lock = lock_of(&[("450814997", entry(&[], ""))]);
        let pack = pack_size(&lock, &HashMap::new());

        assert_eq!(by_id(&pack, "450814997").name, "Mod 450814997");
    }

    #[test]
    fn pack_size_zero_bytes_leaves_shares_at_zero() {
        let lock = lock_of(&[("111", entry(&["addons/c.pbo"], "Mod A"))]);
        let pack = pack_size(&lock, &HashMap::new());

        assert_eq!(pack.tracked_bytes, 0);
        assert_eq!(pack.tracked_files, 0);
        assert_eq!(pack.missing_files, 1);
    }

    #[test]
    fn sort_mod_sizes_orders_by_size_descending() {
        let mut mods = vec![
            ModSize {
                id: "1".to_string(),
                name: "Bravo".to_string(),
                pbo_count: 1,
                bytes: 10,
                missing: 0,
            },
            ModSize {
                id: "2".to_string(),
                name: "Alpha".to_string(),
                pbo_count: 1,
                bytes: 30,
                missing: 0,
            },
        ];

        sort_mod_sizes(&mut mods, SortKey::Size);

        assert_eq!(mods[0].name, "Alpha");
        assert_eq!(mods[1].name, "Bravo");
    }

    #[test]
    fn sort_mod_sizes_orders_by_name_case_insensitively() {
        let mut mods = vec![
            ModSize {
                id: "1".to_string(),
                name: "bravo".to_string(),
                pbo_count: 1,
                bytes: 30,
                missing: 0,
            },
            ModSize {
                id: "2".to_string(),
                name: "Alpha".to_string(),
                pbo_count: 1,
                bytes: 10,
                missing: 0,
            },
        ];

        sort_mod_sizes(&mut mods, SortKey::Name);

        assert_eq!(mods[0].name, "Alpha");
        assert_eq!(mods[1].name, "bravo");
    }

    #[test]
    fn pack_size_untracked_keys_sorted_by_size_descending() {
        let lock = lock_of(&[("111", entry(&["addons/a.pbo"], "Mod A"))]);
        let disk = disk_of(&[
            ("addons/a.pbo", 100),
            ("addons/small.pbo", 10),
            ("addons/large.pbo", 500),
            ("addons/mid.pbo", 50),
        ]);

        let pack = pack_size(&lock, &disk);

        assert_eq!(
            pack.untracked_keys,
            vec!["addons/large.pbo", "addons/mid.pbo", "addons/small.pbo"]
        );
        assert_eq!(pack.untracked_count, 3);
        assert_eq!(pack.untracked_bytes, 560);
    }

    #[test]
    fn pack_size_missing_lists_original_paths_once_sorted() {
        let lock = lock_of(&[
            ("111", entry(&["ADDONS\\Z.pbo", "addons/a.pbo"], "Mod A")),
            ("222", entry(&["addons/M.pbo", "ADDONS\\Z.pbo"], "Mod B")),
        ]);
        let disk = disk_of(&[("addons/a.pbo", 100)]);

        let pack = pack_size(&lock, &disk);

        assert_eq!(pack.missing, vec!["ADDONS\\Z.pbo", "addons/M.pbo"]);
        assert_eq!(pack.missing_files, 3);
    }

    #[test]
    fn sources_not_in_lock_excludes_locked_ids_and_sorts_by_name() {
        let lock = lock_of(&[("111", entry(&[], "Locked"))]);
        let mods = vec![
            mod_entry("333", "bravo"),
            mod_entry("111", "Locked"),
            mod_entry("222", "Alpha"),
        ];

        let rows = sources_not_in_lock(&lock, &mods);

        assert_eq!(
            rows,
            vec![
                ("Alpha".to_string(), "222".to_string()),
                ("bravo".to_string(), "333".to_string()),
            ]
        );
    }
}
