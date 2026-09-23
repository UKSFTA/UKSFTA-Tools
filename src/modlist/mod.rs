mod deps;
mod html;
mod import;
mod legacy;
mod migrate;
mod parse_html;
mod toml_sources;

use std::fs;
use std::path::Path;

use crate::error::UksftaError;
use crate::util::extract_id;

use legacy::parse_legacy_mod_sources;
use toml_sources::TomlModSources;

pub use deps::{parse_required_items, print_dep_tree, resolve_transitive_deps};
pub use html::{generate_modlist, modlist_row_html};
pub use import::import_modlist;
pub use legacy::toml_from_legacy;
pub use migrate::migrate_legacy_if_needed;
pub use parse_html::parse_modlist_html;

// --- Mod list parsing ---

#[derive(Debug, Clone)]
pub struct ModEntry {
    pub id: String,
    pub name: String,
    pub tags: Vec<String>,
    pub role: String,
    pub enabled: bool,
    pub dependencies: Vec<String>,
}

/// Parse mod_sources.txt in either TOML (v2) or legacy (v1) format.
/// Returns (mods, ignored ids). Migration is a separate, explicit step
/// (`migrate_legacy_if_needed`) so read-only commands never write.
pub fn parse_mod_sources(path: &Path) -> Result<(Vec<ModEntry>, Vec<String>), UksftaError> {
    let content = fs::read_to_string(path).map_err(|_| {
        UksftaError::Input(
            "no mod_sources.txt found in this directory\n\
             Run from a mod repo root containing mod_sources.txt"
                .to_string(),
        )
    })?;

    let (mods, ignored, _is_legacy) = parse_mod_sources_content(&content)?;
    Ok((mods, ignored))
}

/// Parse mod_sources.txt content. Third return value is true when the input
/// was legacy (v1) format, so the caller can migrate it.
pub fn parse_mod_sources_content(
    content: &str,
) -> Result<(Vec<ModEntry>, Vec<String>, bool), UksftaError> {
    let looks_toml = content.contains("[[mods]]")
        || content
            .lines()
            .any(|l| l.trim_start().starts_with("version ="));
    if looks_toml {
        let src = toml::from_str::<TomlModSources>(content).map_err(|e| {
            UksftaError::Parse(format!("failed to parse TOML mod_sources.txt: {e}"))
        })?;
        let mut mods = Vec::new();
        let mut ignored = Vec::new();
        for m in src.mods {
            // The id field accepts a bare ID or a full Workshop URL
            let Some(id) = extract_id(&m.id) else {
                return Err(UksftaError::Parse(format!(
                    "invalid mod id in mod_sources.txt: {}",
                    m.id
                )));
            };
            let name = if m.name.is_empty() {
                format!("Mod {}", id)
            } else {
                m.name
            };
            let entry = ModEntry {
                id,
                name,
                tags: m.tags,
                role: m.role,
                enabled: m.enabled,
                dependencies: m.dependencies,
            };
            if entry.role == "ignore" || !entry.enabled {
                ignored.push(entry.id.clone());
            } else {
                mods.push(entry);
            }
        }
        Ok((mods, ignored, false))
    } else {
        let (mods, ignored) = parse_legacy_mod_sources(content);
        Ok((mods, ignored, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_toml_mod_sources() {
        let toml = r#"version = 2

[[mods]]
id = "450814997"
name = "CBA_A3"

[[mods]]
id = "887302721"
name = "Boat Mod"
tags = ["vehicles"]
dependencies = ["450814997"]

[[mods]]
id = "463939057"
role = "ignore"
enabled = false
"#;
        let (mods, ignored, is_legacy) = parse_mod_sources_content(toml).unwrap();
        assert!(!is_legacy);
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].id, "450814997");
        assert_eq!(mods[0].name, "CBA_A3");
        assert_eq!(mods[1].tags, vec!["vehicles".to_string()]);
        assert_eq!(mods[1].dependencies, vec!["450814997".to_string()]);
        assert_eq!(ignored, vec!["463939057".to_string()]);
    }

    #[test]
    fn parse_toml_without_version_field() {
        // version defaults to 2 when absent
        let toml = r#"[[mods]]
id = "1234567890"
name = "Some Mod"
"#;
        let (mods, ignored, is_legacy) = parse_mod_sources_content(toml).unwrap();
        assert!(!is_legacy);
        assert_eq!(mods.len(), 1);
        assert!(ignored.is_empty());
    }

    #[test]
    fn parse_toml_disabled_mod_is_ignored() {
        let toml = r#"[[mods]]
id = "1234567890"
enabled = false
"#;
        let (mods, ignored, _) = parse_mod_sources_content(toml).unwrap();
        assert!(mods.is_empty());
        assert_eq!(ignored, vec!["1234567890".to_string()]);
    }

    #[test]
    fn parse_toml_missing_name_defaults_to_mod_id() {
        let toml = r#"[[mods]]
id = "1234567890"
"#;
        let (mods, _, _) = parse_mod_sources_content(toml).unwrap();
        assert_eq!(mods[0].name, "Mod 1234567890");
    }

    #[test]
    fn parse_legacy_still_works() {
        let legacy = "450814997 # CBA_A3\n887302721 # Boat Mod\n\n[ignore]\n463939057 # ACE\n";
        let (mods, ignored, is_legacy) = parse_mod_sources_content(legacy).unwrap();
        assert!(is_legacy);
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].id, "450814997");
        assert_eq!(mods[0].name, "CBA_A3");
        assert_eq!(ignored, vec!["463939057".to_string()]);
    }

    #[test]
    fn parse_toml_accepts_workshop_url_in_id() {
        let toml = r#"[[mods]]
id = "https://steamcommunity.com/sharedfiles/filedetails/?id=450814997"
name = "CBA_A3"
"#;
        let (mods, _, is_legacy) = parse_mod_sources_content(toml).unwrap();
        assert!(!is_legacy);
        assert_eq!(mods[0].id, "450814997");
        assert_eq!(mods[0].name, "CBA_A3");
    }

    #[test]
    fn parse_toml_rejects_invalid_id() {
        // Invalid id (no 8+ digit ID, no URL) must error out. We simulate by
        // checking extract_id directly since it decides the parse result.
        assert_eq!(extract_id("not-a-mod"), None);
        assert_eq!(extract_id("123"), None);
    }
}
