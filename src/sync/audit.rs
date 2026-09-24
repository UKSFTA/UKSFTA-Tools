use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use crate::error::UksftaError;
use crate::lock::{load_lock, LockFile};
use crate::modlist::{parse_mod_sources, ModEntry};
use crate::steam::{find_all_workshop_caches, find_mod_in_caches};
use crate::util::find_pbos;

/// One thing the audit checks: an enabled root mod, or a dependency id that
/// a root declares in its `dependencies` array.
pub(crate) struct AuditTarget {
    pub id: String,
    pub name: String,
    pub is_dependency: bool,
}

/// Build the audit set. Roots come first in list order, then every declared
/// dependency id that is not ignored and not already a root, sorted and
/// de-duplicated. A dependency name comes from the lock when it holds a
/// non-empty name, otherwise `Mod <id>`.
pub(crate) fn audit_targets(
    mods: &[ModEntry],
    ignored: &[String],
    lock: &LockFile,
) -> Vec<AuditTarget> {
    let ignored_set: HashSet<&str> = ignored.iter().map(String::as_str).collect();
    let root_ids: HashSet<&str> = mods.iter().map(|m| m.id.as_str()).collect();

    let mut targets: Vec<AuditTarget> = mods
        .iter()
        .map(|m| AuditTarget {
            id: m.id.clone(),
            name: m.name.clone(),
            is_dependency: false,
        })
        .collect();

    let mut dep_ids: BTreeSet<&str> = BTreeSet::new();
    for entry in mods {
        for id in &entry.dependencies {
            let id = id.as_str();
            if !ignored_set.contains(id) && !root_ids.contains(id) {
                dep_ids.insert(id);
            }
        }
    }

    for id in dep_ids {
        let name = lock
            .mods
            .get(id)
            .filter(|e| !e.name.is_empty())
            .map(|e| e.name.clone())
            .unwrap_or_else(|| format!("Mod {id}"));
        targets.push(AuditTarget {
            id: id.to_string(),
            name,
            is_dependency: true,
        });
    }

    targets
}

/// Audit each target's PBOs against addons/ (present/missing per PBO).
pub fn audit(missing_only: bool) -> Result<(), UksftaError> {
    let (mods, ignored) = parse_mod_sources(Path::new("mod_sources.txt"))?;
    if mods.is_empty() {
        return Err(UksftaError::Input(
            "No mods found in mod_sources.txt".to_string(),
        ));
    }

    // Audit runs before the first sync, so a missing lock is an empty lock.
    let lock = load_lock(Path::new("mods.lock"))?;
    let targets = audit_targets(&mods, &ignored, &lock);
    let root_count = targets.iter().filter(|t| !t.is_dependency).count();
    let dependency_count = targets.len() - root_count;

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

    // Collect every expected PBO name across all targets, for orphan detection
    let mut all_expected: HashMap<String, String> = HashMap::new(); // pbo name -> mod name
    let mut not_in_cache = 0;
    let mut total_missing = 0;
    let mut total_expected = 0;
    let mut missing_list: Vec<String> = Vec::new();

    println!("--- Mod audit ---");

    for target in &targets {
        let marker = if target.is_dependency {
            " [dependency]"
        } else {
            ""
        };

        // Find this target in any cache
        let mod_path = match find_mod_in_caches(&caches, &target.id) {
            Some(p) => p,
            None => {
                println!(
                    "  [NOT IN CACHE] {} ({}){} — cannot enumerate PBOs",
                    target.name, target.id, marker
                );
                not_in_cache += 1;
                continue;
            }
        };

        // Expected PBOs from the cache, by filename
        let expected = find_pbos(&mod_path);
        if expected.is_empty() {
            println!(
                "  [EMPTY]   {} ({}){} — no PBOs in cache",
                target.name, target.id, marker
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
                .or_insert_with(|| target.name.clone());
        }

        let status = if missing.is_empty() {
            "OK"
        } else {
            "INCOMPLETE"
        };
        if !missing_only || !missing.is_empty() {
            println!(
                "  [{}] {} ({}){} — {}/{} PBOs",
                status,
                target.name,
                target.id,
                marker,
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
            missing_list.push(format!("{} -> {}", name, target.name));
        }
        total_missing += missing.len();
    }

    // Orphan detection: PBOs in addons/ that belong to no expected target
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
        "\nSummary: {}/{} PBOs present ({}%) across {} mods ({} root, {} dependency); {} not in cache",
        total_expected - total_missing,
        total_expected,
        pct,
        targets.len(),
        root_count,
        dependency_count,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::ModLockEntry;
    use std::collections::HashMap;

    fn root(id: &str, name: &str, deps: &[&str]) -> ModEntry {
        ModEntry {
            id: id.to_string(),
            name: name.to_string(),
            tags: Vec::new(),
            role: "mod".to_string(),
            enabled: true,
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn empty_lock() -> LockFile {
        LockFile {
            version: 1,
            mods: HashMap::new(),
        }
    }

    fn lock_entry(name: &str) -> ModLockEntry {
        ModLockEntry {
            files: Vec::new(),
            name: name.to_string(),
            tags: Vec::new(),
            dependencies: Vec::new(),
            updated: String::new(),
        }
    }

    #[test]
    fn root_and_declared_dependency_yield_two_targets() {
        let mods = vec![root("111", "Root", &["450814997"])];

        let targets = audit_targets(&mods, &[], &empty_lock());

        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].id, "111");
        assert!(!targets[0].is_dependency);
        assert_eq!(targets[1].id, "450814997");
        assert!(targets[1].is_dependency);
    }

    #[test]
    fn ignored_dependency_is_excluded() {
        let mods = vec![root("111", "Root", &["450814997"])];

        let targets = audit_targets(&mods, &["450814997".to_string()], &empty_lock());

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "111");
    }

    #[test]
    fn dependency_that_is_also_a_root_appears_once_as_root() {
        let mods = vec![
            root("111", "Root", &["450814997"]),
            root("450814997", "CBA_A3", &[]),
        ];

        let targets = audit_targets(&mods, &[], &empty_lock());

        assert_eq!(targets.len(), 2);
        assert!(targets.iter().all(|t| !t.is_dependency));
        assert_eq!(targets[1].id, "450814997");
        assert_eq!(targets[1].name, "CBA_A3");
    }

    #[test]
    fn dependency_name_uses_lock_name_when_present() {
        let mods = vec![root("111", "Root", &["450814997", "843425103"])];
        let mut lock = empty_lock();
        lock.mods
            .insert("450814997".to_string(), lock_entry("Community Base Addons"));
        lock.mods.insert("843425103".to_string(), lock_entry(""));

        let targets = audit_targets(&mods, &[], &lock);

        assert_eq!(targets[1].name, "Community Base Addons");
        assert_eq!(targets[2].name, "Mod 843425103");
    }
}
