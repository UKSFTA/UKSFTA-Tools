use std::fs;
use std::path::Path;

use super::toml_sources::{to_toml_string, TomlMod, TomlModSources};
use crate::error::UksftaError;
use crate::util::{extract_id, workshop_url};

#[cfg(test)]
use super::ModEntry;

/// Convert a parsed entry back into its TOML form. The synthetic
/// `Mod <id>` name is dropped so it is not written out.
#[cfg(test)]
pub(crate) fn toml_mod_from_entry(entry: &ModEntry) -> TomlMod {
    let name = if entry.name == format!("Mod {}", entry.id) {
        String::new()
    } else {
        entry.name.clone()
    };
    TomlMod {
        id: workshop_url(&entry.id),
        name,
        tags: entry.tags.clone(),
        role: entry.role.clone(),
        enabled: entry.enabled,
        dependencies: entry.dependencies.clone(),
    }
}

#[cfg(test)]
pub(crate) fn serialize_mod_entries(entries: &[ModEntry]) -> Result<String, UksftaError> {
    let doc = TomlModSources {
        version: 2,
        mods: entries.iter().map(toml_mod_from_entry).collect(),
    };
    to_toml_string(&doc)
}

/// Persist ignored entries into `mod_sources.txt`. An id already present in
/// any block is left untouched, so a later plain `sync` ignores it. The
/// write is atomic with a `.bak` backup. Returns true when the file was
/// rewritten.
pub fn persist_ignored(path: &Path, entries: &[(String, String)]) -> Result<bool, UksftaError> {
    if entries.is_empty() {
        return Ok(false);
    }

    let content = fs::read_to_string(path)?;
    let mut doc: TomlModSources = toml::from_str(&content)
        .map_err(|e| UksftaError::Parse(format!("failed to parse TOML mod_sources.txt: {e}")))?;

    let existing: std::collections::HashSet<String> =
        doc.mods.iter().filter_map(|m| extract_id(&m.id)).collect();

    let mut changed = false;
    for (id, name) in entries {
        if existing.contains(id) {
            continue;
        }
        doc.mods.push(TomlMod {
            id: workshop_url(id),
            name: name.clone(),
            tags: Vec::new(),
            role: "ignore".to_string(),
            enabled: false,
            dependencies: Vec::new(),
        });
        changed = true;
    }

    if changed {
        crate::atomic::write_atomic_with_backup(path, to_toml_string(&doc)?.as_bytes())?;
    }
    Ok(changed)
}

/// Write discovered dependency lists into the matching `[[mods]]` blocks
/// of `path`. The write is atomic with a `.bak` backup and happens only
/// when a value changed. Returns true when the file was rewritten.
///
/// Parsing the raw document and editing it in place preserves every other
/// field and every ignored block byte-for-byte in value.
pub fn persist_dependencies(
    path: &Path,
    updates: &[(String, Vec<String>)],
) -> Result<bool, UksftaError> {
    if updates.is_empty() {
        return Ok(false);
    }

    let content = fs::read_to_string(path)?;
    let mut doc: TomlModSources = toml::from_str(&content)
        .map_err(|e| UksftaError::Parse(format!("failed to parse TOML mod_sources.txt: {e}")))?;

    let mut changed = false;
    for m in &mut doc.mods {
        let Some(id) = extract_id(&m.id) else {
            continue;
        };
        let Some((_, deps)) = updates.iter().find(|(root, _)| root == &id) else {
            continue;
        };
        if &m.dependencies != deps {
            m.dependencies = deps.clone();
            changed = true;
        }
    }

    if changed {
        crate::atomic::write_atomic_with_backup(path, to_toml_string(&doc)?.as_bytes())?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::parse_mod_sources_content;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("uksfta-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_entry() -> ModEntry {
        ModEntry {
            id: "450814997".to_string(),
            name: "CBA_A3".to_string(),
            tags: vec!["core".to_string()],
            role: "mod".to_string(),
            enabled: true,
            dependencies: vec!["1111111111".to_string()],
        }
    }

    #[test]
    fn serialize_mod_entries_round_trips() {
        let ignored_entry = ModEntry {
            id: "463939057".to_string(),
            name: "ACE".to_string(),
            tags: Vec::new(),
            role: "ignore".to_string(),
            enabled: false,
            dependencies: Vec::new(),
        };
        let toml = serialize_mod_entries(&[sample_entry(), ignored_entry]).unwrap();

        let (mods, ignored, is_legacy) = parse_mod_sources_content(&toml).unwrap();
        assert!(!is_legacy);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].id, "450814997");
        assert_eq!(mods[0].name, "CBA_A3");
        assert_eq!(mods[0].tags, vec!["core".to_string()]);
        assert_eq!(mods[0].dependencies, vec!["1111111111".to_string()]);
        assert_eq!(ignored, vec!["463939057".to_string()]);

        // The ignored block keeps its role and enabled flag in the document.
        let doc: TomlModSources = toml::from_str(&toml).unwrap();
        let block = doc
            .mods
            .iter()
            .find(|m| m.id.contains("463939057"))
            .unwrap();
        assert_eq!(block.role, "ignore");
        assert!(!block.enabled);
    }

    #[test]
    fn persist_dependencies_updates_target_and_preserves_ignored() {
        let dir = temp_dir("persist");
        let path = dir.join("mod_sources.txt");
        let original = r#"version = 2

[[mods]]
id = "9999999999"
name = "Root"

[[mods]]
id = "450814997"
name = "CBA_A3"
role = "ignore"
enabled = false
"#;
        std::fs::write(&path, original).unwrap();

        let updates = vec![("9999999999".to_string(), vec!["450814997".to_string()])];
        assert!(persist_dependencies(&path, &updates).unwrap());

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: TomlModSources = toml::from_str(&content).unwrap();
        let root = doc.mods.iter().find(|m| m.name == "Root").unwrap();
        assert_eq!(root.dependencies, vec!["450814997".to_string()]);
        let ignored = doc.mods.iter().find(|m| m.name == "CBA_A3").unwrap();
        assert_eq!(ignored.role, "ignore");
        assert!(!ignored.enabled);
    }

    #[test]
    fn persist_dependencies_no_write_when_unchanged() {
        let dir = temp_dir("persist-noop");
        let path = dir.join("mod_sources.txt");
        let original = r#"version = 2

[[mods]]
id = "9999999999"
name = "Root"
dependencies = ["450814997"]
"#;
        std::fs::write(&path, original).unwrap();
        let before = std::fs::read(&path).unwrap();

        let updates = vec![("9999999999".to_string(), vec!["450814997".to_string()])];
        assert!(!persist_dependencies(&path, &updates).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn persist_ignored_adds_an_ignore_block() {
        let dir = temp_dir("ignore-add");
        let path = dir.join("mod_sources.txt");
        let original = "version = 2\n\n[[mods]]\nid = \"9999999999\"\nname = \"Root\"\n";
        std::fs::write(&path, original).unwrap();

        let entries = vec![("450814997".to_string(), "CBA_A3".to_string())];
        assert!(persist_ignored(&path, &entries).unwrap());

        let content = std::fs::read_to_string(&path).unwrap();
        let doc: TomlModSources = toml::from_str(&content).unwrap();
        let added = doc
            .mods
            .iter()
            .find(|m| m.id.contains("450814997"))
            .unwrap();
        assert_eq!(added.role, "ignore");
        assert!(!added.enabled);
        assert_eq!(added.name, "CBA_A3");
    }

    #[test]
    fn persist_ignored_does_not_duplicate_an_existing_id() {
        let dir = temp_dir("ignore-dup");
        let path = dir.join("mod_sources.txt");
        let original = "version = 2\n\n[[mods]]\nid = \"450814997\"\nname = \"CBA_A3\"\nrole = \"ignore\"\nenabled = false\n";
        std::fs::write(&path, original).unwrap();
        let before = std::fs::read(&path).unwrap();

        let entries = vec![("450814997".to_string(), "CBA_A3".to_string())];
        assert!(!persist_ignored(&path, &entries).unwrap());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn persist_ignored_skips_a_known_root_id() {
        let dir = temp_dir("ignore-root");
        let path = dir.join("mod_sources.txt");
        let original = "version = 2\n\n[[mods]]\nid = \"9999999999\"\nname = \"Root\"\n";
        std::fs::write(&path, original).unwrap();

        let entries = vec![("9999999999".to_string(), "Root".to_string())];
        assert!(!persist_ignored(&path, &entries).unwrap());
    }
}
