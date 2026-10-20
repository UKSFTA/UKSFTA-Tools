//! Tests for composition diffs and changelog rendering.

use super::super::state::{ModMap, PublishState};
use super::diff::{change, change_with_update, diff, ChangeSet};
use super::*;
use crate::lock::{LockFile, ModLockEntry};

fn mods(items: &[(&str, &str, &str)]) -> ModMap {
    items
        .iter()
        .map(|(id, name, updated)| {
            (
                id.to_string(),
                super::super::state::ModSnapshot {
                    name: name.to_string(),
                    updated: updated.to_string(),
                },
            )
        })
        .collect()
}

fn state(version: &str, baseline: ModMap, last: ModMap) -> PublishState {
    PublishState {
        schema: 1,
        project_version: version.to_string(),
        baseline,
        last,
        initial: None,
    }
}

#[test]
fn snapshot_keeps_only_installed_mods() {
    let mut lock = LockFile {
        version: 1,
        mods: Default::default(),
    };
    lock.mods.insert(
        "1".into(),
        ModLockEntry {
            files: vec!["addons/a.pbo".into()],
            name: "Installed".into(),
            tags: Vec::new(),
            dependencies: Vec::new(),
            updated: "100".into(),
        },
    );
    lock.mods.insert(
        "2".into(),
        ModLockEntry {
            files: Vec::new(),
            name: "Not installed".into(),
            tags: Vec::new(),
            dependencies: Vec::new(),
            updated: "100".into(),
        },
    );
    let map = snapshot(&lock);
    assert_eq!(map.len(), 1);
    assert_eq!(map["1"].name, "Installed");
}

#[test]
fn diff_reports_added_updated_and_removed() {
    let before = mods(&[
        ("1", "Keep", "100"),
        ("2", "Update", "100"),
        ("3", "Remove", "100"),
    ]);
    let after = mods(&[
        ("1", "Keep", "100"),
        ("2", "Update", "200"),
        ("4", "New", "300"),
    ]);
    let changes = diff(&before, &after);
    assert_eq!(changes.added, vec![change("4", "New".into())]);
    assert_eq!(
        changes.updated,
        vec![change_with_update("2", "Update".into(), "200".into())]
    );
    assert_eq!(changes.removed, vec![change("3", "Remove".into())]);
}

#[test]
fn rename_counts_as_an_update() {
    let before = mods(&[("1", "Old name", "100")]);
    let after = mods(&[("1", "New name", "100")]);
    assert_eq!(
        diff(&before, &after).updated,
        vec![change_with_update("1", "New name".into(), "100".into())]
    );
}

#[test]
fn changed_items_show_the_workshop_update_date() {
    let changes = Changes::diff(ChangeSet {
        updated: vec![change_with_update("1", "Alpha".into(), "1700000000".into())],
        ..ChangeSet::default()
    });
    let text = render_changelog(None, "1.0.0.0", "2026-10-20", &changes);
    assert!(text.contains("- Alpha (Workshop ID: 1), updated 2023-11-14"));
}

#[test]
fn generated_sections_are_replaceable_but_hand_written_ones_are_not() {
    let generated = render_changelog(
        None,
        "1.0.0.0",
        "2026-10-20",
        &Changes::diff(ChangeSet::default()),
    );
    assert!(can_replace(Some(&generated), "1.0.0.0"));
    let hand_written =
        "# Changelog\n\n## [1.0.0.0] - 2026-01-01\n\n### Added\n\n- Written by hand\n";
    assert!(!can_replace(Some(hand_written), "1.0.0.0"));
    assert!(can_replace(Some(hand_written), "2.0.0.0"));
}

#[test]
fn first_run_records_an_initial_release() {
    let text = render_changelog(None, "1.0.0.0", "2026-10-20", &Changes::initial(3));
    assert!(text.starts_with("# Changelog\n"));
    assert!(text.contains("## [1.0.0.0] - 2026-10-20"));
    assert!(text.contains("- Initial release with 3 mods."));
}

#[test]
fn renders_changes_in_keep_a_changelog_order() {
    let changes = ChangeSet {
        added: vec![change("1", "Alpha".into())],
        updated: vec![change("2", "Bravo".into())],
        removed: vec![change("3", "Charlie".into())],
    };
    let text = render_changelog(None, "1.0.0.0", "2026-10-20", &Changes::diff(changes));
    let added = text.find("### Added").unwrap();
    let changed = text.find("### Changed").unwrap();
    let removed = text.find("### Removed").unwrap();
    assert!(added < changed && changed < removed);
    assert!(text.contains("- Alpha (Workshop ID: 1)"));
    assert!(text.contains("- Bravo (Workshop ID: 2)"));
    assert!(text.contains("- Charlie (Workshop ID: 3)"));
}

#[test]
fn replaces_the_current_version_and_keeps_older_sections() {
    let existing = "# Changelog\n\n## [1.0.0.0] - 2026-01-01\n\n### Added\n\n- Old\n\n## [0.9.0.0] - 2025-12-01\n\n### Added\n\n- Ancient\n";
    let changes = Changes::diff(ChangeSet {
        added: vec![change("1", "New".into())],
        ..ChangeSet::default()
    });
    let text = render_changelog(Some(existing), "1.0.0.0", "2026-10-20", &changes);
    assert!(text.contains("## [1.0.0.0] - 2026-01-01"));
    assert!(text.contains("- New (Workshop ID: 1)"));
    assert!(!text.contains("- Old"));
    assert!(text.contains("## [0.9.0.0] - 2025-12-01"));
    assert!(text.contains("- Ancient"));
}

#[test]
fn inserts_a_new_version_before_older_sections() {
    let existing = "# Changelog\n\n## [1.0.0.0] - 2026-01-01\n\n### Added\n\n- Old\n";
    let changes = Changes::diff(ChangeSet {
        added: vec![change("1", "New".into())],
        ..ChangeSet::default()
    });
    let text = render_changelog(Some(existing), "1.1.0.0", "2026-10-20", &changes);
    assert!(text.find("## [1.1.0.0]").unwrap() < text.find("## [1.0.0.0]").unwrap());
}

#[test]
fn a_version_prefix_does_not_match_a_longer_version() {
    let existing = "# Changelog\n\n## [1.0.0.0] - 2026-01-01\n\n### Added\n\n- Old\n";
    let text = render_changelog(
        Some(existing),
        "1.0.0",
        "2026-10-20",
        &Changes::diff(ChangeSet::default()),
    );
    assert!(text.contains("## [1.0.0.0] - 2026-01-01"));
    assert!(text.contains("## [1.0.0] - 2026-10-20"));
}

#[test]
fn empty_diff_records_a_note() {
    let text = render_changelog(
        None,
        "2.0.0.0",
        "2026-10-20",
        &Changes::diff(ChangeSet::default()),
    );
    assert!(text.contains("_No changes recorded._"));
}

#[test]
fn first_run_state_uses_the_current_composition_as_the_baseline() {
    let current = mods(&[("1", "Alpha", "100")]);
    let next = next_state(None, "1.0.0.0", &current);
    assert_eq!(next.baseline, current);
    assert_eq!(next.last, current);
    assert_eq!(changes_for(None, "1.0.0.0", &current), Changes::initial(1));
}

#[test]
fn same_version_keeps_the_baseline_and_accumulates() {
    let baseline = mods(&[("1", "Alpha", "100")]);
    let previous = state("1.0.0.0", baseline.clone(), baseline.clone());
    let current = mods(&[("1", "Alpha", "100"), ("2", "Bravo", "200")]);
    let changes = changes_for(Some(&previous), "1.0.0.0", &current);
    assert_eq!(
        changes,
        Changes::diff(ChangeSet {
            added: vec![change("2", "Bravo".into())],
            ..ChangeSet::default()
        })
    );
    let next = next_state(Some(&previous), "1.0.0.0", &current);
    assert_eq!(next.baseline, baseline);
    assert_eq!(next.last, current);
}

#[test]
fn a_new_version_rebaselines_at_the_previous_run() {
    let baseline = mods(&[("1", "Alpha", "100")]);
    let last = mods(&[("1", "Alpha", "100"), ("2", "Bravo", "200")]);
    let previous = state("1.0.0.0", baseline, last.clone());
    let current = mods(&[("2", "Bravo", "200")]);
    let changes = changes_for(Some(&previous), "1.1.0.0", &current);
    assert_eq!(
        changes,
        Changes::diff(ChangeSet {
            removed: vec![change("1", "Alpha".into())],
            ..ChangeSet::default()
        })
    );
    let next = next_state(Some(&previous), "1.1.0.0", &current);
    assert_eq!(next.baseline, last);
}
