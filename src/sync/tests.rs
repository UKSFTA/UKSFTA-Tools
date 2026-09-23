use std::collections::HashMap;

use super::{lookup_sizes, missing_size_ids, size_share_colour};
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
