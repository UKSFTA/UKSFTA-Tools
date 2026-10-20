use super::*;
use crate::lock::{Dependency, ModLockEntry};

fn entry(name: &str, dependencies: Vec<Dependency>) -> ModLockEntry {
    ModLockEntry {
        files: vec!["addons/a.pbo".to_string()],
        name: name.to_string(),
        tags: Vec::new(),
        dependencies,
        updated: String::new(),
    }
}

fn dep(id: &str, name: &str) -> Dependency {
    Dependency {
        id: id.to_string(),
        name: name.to_string(),
    }
}

#[test]
fn finds_placeholders_blanks_and_dependencies() {
    let mut lock = LockFile {
        version: 1,
        mods: Default::default(),
    };
    lock.mods.insert(
        "1".into(),
        entry("Mod 1", vec![dep("9", ""), dep("8", "Named")]),
    );
    lock.mods.insert("2".into(), entry("Real", Vec::new()));
    assert_eq!(missing_name_ids(&lock), vec!["1", "9"]);
}

#[test]
fn applies_only_to_missing_names() {
    let mut lock = LockFile {
        version: 1,
        mods: Default::default(),
    };
    lock.mods.insert(
        "1".into(),
        entry("Mod 1", vec![dep("9", ""), dep("8", "Keep")]),
    );
    let names = HashMap::from([
        ("1".to_string(), "Alpha".to_string()),
        ("9".to_string(), "Bravo".to_string()),
        ("8".to_string(), "Replaced".to_string()),
    ]);
    assert_eq!(apply_names(&mut lock, &names), 2);
    assert_eq!(lock.mods["1"].name, "Alpha");
    assert_eq!(lock.mods["1"].dependencies[0].name, "Bravo");
    assert_eq!(lock.mods["1"].dependencies[1].name, "Keep");
}

#[test]
fn empty_names_are_filled() {
    let mut lock = LockFile {
        version: 1,
        mods: Default::default(),
    };
    lock.mods.insert("5".into(), entry("", Vec::new()));
    let names = HashMap::from([("5".to_string(), "Echo".to_string())]);
    assert_eq!(apply_names(&mut lock, &names), 1);
    assert_eq!(lock.mods["5"].name, "Echo");
}

#[test]
fn cache_round_trips() {
    let path = std::env::temp_dir().join(format!("uksfta-names-{}", std::process::id()));
    let _ = fs::remove_file(&path);
    let names = HashMap::from([("1".to_string(), "Alpha".to_string())]);
    save_cache(&path, &names).unwrap();
    assert_eq!(load_cache(&path).unwrap(), names);
    let _ = fs::remove_file(&path);
}

#[test]
fn missing_cache_is_empty() {
    let path = std::env::temp_dir().join(format!("uksfta-names-none-{}", std::process::id()));
    let _ = fs::remove_file(&path);
    assert!(load_cache(&path).unwrap().is_empty());
}

#[test]
fn refresh_fetches_cached_placeholders_again() {
    let mut lock = LockFile {
        version: 1,
        mods: Default::default(),
    };
    lock.mods.insert("1".into(), entry("Mod 1", Vec::new()));
    lock.mods.insert("2".into(), entry("Real", Vec::new()));
    let cache = HashMap::from([("1".to_string(), "Old title".to_string())]);
    assert!(ids_to_fetch(&lock, &cache, false).is_empty());
    assert_eq!(ids_to_fetch(&lock, &cache, true), vec!["1"]);
}

#[test]
fn pruning_drops_ids_that_left_the_lock() {
    let mut lock = LockFile {
        version: 1,
        mods: Default::default(),
    };
    lock.mods
        .insert("1".into(), entry("Alpha", vec![dep("9", "")]));
    let mut cache = HashMap::from([
        ("1".to_string(), "Alpha".to_string()),
        ("9".to_string(), "Dep".to_string()),
        ("2".to_string(), "Gone".to_string()),
    ]);
    assert_eq!(prune_cache(&mut cache, &lock), 1);
    assert!(!cache.contains_key("2"));
    assert_eq!(cache.len(), 2);
}
