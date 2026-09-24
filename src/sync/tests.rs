use std::collections::HashMap;

use super::plan::{build_plan, SyncPlan};
use super::{lookup_sizes, missing_size_ids, size_share_colour, unpullable_dependencies};
use crate::lock::{LockFile, ModLockEntry};
use crate::modlist::{DepMap, ModEntry};
use crate::steam::WorkshopItem;
use crate::util::{C_GREEN, C_RED, C_YELLOW};

// --- lookup_sizes ---

#[test]
fn lookup_sizes_marks_missing_mods_as_unknown() {
    let mods = vec![
        ("111".to_string(), "Mod A".to_string()),
        ("222".to_string(), "Mod B".to_string()),
    ];
    let mut items = HashMap::new();
    items.insert(
        "111".to_string(),
        WorkshopItem {
            size: 500,
            time_updated: 0,
        },
    );
    let sizes = lookup_sizes(&mods, &items);
    assert_eq!(
        sizes[0],
        ("111".to_string(), "Mod A".to_string(), Some(500))
    );
    assert_eq!(sizes[1], ("222".to_string(), "Mod B".to_string(), None));
}

#[test]
fn missing_size_ids_skips_known_and_dedupes() {
    let mods = vec![
        ("111".to_string(), "Mod A".to_string()),
        ("222".to_string(), "Mod B".to_string()),
        ("222".to_string(), "Mod B again".to_string()),
        ("333".to_string(), "Mod C".to_string()),
    ];
    let mut items = HashMap::new();
    items.insert(
        "111".to_string(),
        WorkshopItem {
            size: 500,
            time_updated: 0,
        },
    );
    // 333 has a zero size, which counts as unknown and is included.
    items.insert(
        "333".to_string(),
        WorkshopItem {
            size: 0,
            time_updated: 0,
        },
    );
    assert_eq!(missing_size_ids(&mods, &items), vec!["222", "333"]);
}

#[test]
fn size_share_colour_grades_by_share() {
    assert_eq!(size_share_colour(0.0), C_GREEN);
    assert_eq!(size_share_colour(0.049), C_GREEN);
    assert_eq!(size_share_colour(0.05), C_YELLOW);
    assert_eq!(size_share_colour(0.199), C_YELLOW);
    assert_eq!(size_share_colour(0.20), C_RED);
    assert_eq!(size_share_colour(0.9), C_RED);
}

#[test]
fn dependency_survives_plain_sync() {
    let root = std::env::temp_dir().join(format!("uksfta-sync-{}", std::process::id()));
    let cache = root.join("workshop").join("content").join("107410");
    let dep_dir = cache.join("450814997");
    std::fs::create_dir_all(&dep_dir).unwrap();
    std::fs::write(dep_dir.join("x.pbo"), b"pbo").unwrap();

    let mods = vec![ModEntry {
        id: "9999999999".to_string(),
        name: "Root".to_string(),
        tags: Vec::new(),
        role: "mod".to_string(),
        enabled: true,
        dependencies: vec!["450814997".to_string()],
    }];
    let mut lock = LockFile {
        version: 1,
        mods: HashMap::new(),
    };
    lock.mods.insert(
        "450814997".to_string(),
        ModLockEntry {
            files: vec!["addons/x.pbo".to_string()],
            name: "CBA_A3".to_string(),
            tags: Vec::new(),
            dependencies: Vec::new(),
            updated: "0".to_string(),
        },
    );

    let plan = build_plan(
        &mods,
        &[],
        std::slice::from_ref(&cache),
        &lock,
        &HashMap::new(),
        &DepMap::new(),
    );

    assert!(plan.planned.iter().any(|p| p.id == "450814997"));
    assert!(!plan.removed.iter().any(|r| r.id == "450814997"));
    assert!(plan.required_deps.contains("450814997"));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn unpullable_dependencies_flags_only_required_deps() {
    let plan = SyncPlan {
        planned: Vec::new(),
        missing_from_cache: vec![
            ("9999999999".to_string(), "Root".to_string()),
            ("450814997".to_string(), "CBA_A3".to_string()),
        ],
        removed: Vec::new(),
        added: 0,
        updated: 0,
        unchanged: 0,
        discovered_deps: DepMap::new(),
        required_deps: ["450814997".to_string()].into_iter().collect(),
    };
    assert_eq!(unpullable_dependencies(&plan), vec!["450814997"]);
}
