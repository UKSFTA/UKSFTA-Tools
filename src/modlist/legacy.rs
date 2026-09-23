use super::toml_sources::{TomlMod, TomlModSources};
use super::ModEntry;
use crate::error::UksftaError;
use crate::util::{extract_id, extract_tag, workshop_url};

/// True when a legacy line is exactly the `[ignore]` or `@ignore` marker.
/// A mod or comment line that merely contains the substring does not match.
pub(crate) fn is_ignore_marker(line: &str) -> bool {
    let normalized = line.trim().to_ascii_lowercase();
    normalized == "[ignore]" || normalized == "@ignore"
}

/// Legacy v1 parser: one Workshop mod per line, optional `# tag`,
/// optional `[ignore]` / `@ignore` section at the end.
pub(crate) fn parse_legacy_mod_sources(content: &str) -> (Vec<ModEntry>, Vec<String>) {
    let mut mods = Vec::new();
    let mut ignored = Vec::new();
    let mut in_ignore = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            if is_ignore_marker(line) {
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
        if is_ignore_marker(line) {
            in_ignore = true;
            continue;
        }
        if let Some(id) = extract_id(line) {
            let name = extract_tag(line).unwrap_or_else(|| format!("Mod {}", id));
            mods.push(ModEntry {
                id,
                name,
                tags: Vec::new(),
                role: "mod".to_string(),
                enabled: true,
                dependencies: Vec::new(),
            });
        }
    }
    (mods, ignored)
}

/// Serialise legacy data into the TOML v2 format.
/// Ids are written as full Workshop URLs so entries stay clickable.
pub fn toml_from_legacy(mods: &[ModEntry], ignored: &[String]) -> Result<String, UksftaError> {
    let mut out = TomlModSources {
        version: 2,
        mods: Vec::new(),
    };
    for m in mods {
        let name = if m.name == format!("Mod {}", m.id) {
            String::new()
        } else {
            m.name.clone()
        };
        out.mods.push(TomlMod {
            id: workshop_url(&m.id),
            name,
            tags: Vec::new(),
            role: "mod".to_string(),
            enabled: true,
            dependencies: Vec::new(),
        });
    }
    for id in ignored {
        out.mods.push(TomlMod {
            id: workshop_url(id),
            name: String::new(),
            tags: Vec::new(),
            role: "ignore".to_string(),
            enabled: false,
            dependencies: Vec::new(),
        });
    }
    toml::to_string(&out)
        .map_err(|e| UksftaError::Parse(format!("failed to serialise TOML mod sources: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::parse_mod_sources_content;

    #[test]
    fn is_ignore_marker_requires_exact_line() {
        assert!(is_ignore_marker("[ignore]"));
        assert!(is_ignore_marker("@ignore"));
        assert!(is_ignore_marker("  [Ignore]  "));
        assert!(!is_ignore_marker("450814997 # uses [ignore] tag"));
        assert!(!is_ignore_marker("# [ignore]"));
        assert!(!is_ignore_marker("[ignore] # trailing"));
    }

    #[test]
    fn legacy_parser_keeps_line_containing_ignore_marker() {
        let legacy = "450814997 # uses [ignore] tag\n463939057 # ACE\n";
        let (mods, ignored, is_legacy) = parse_mod_sources_content(legacy).unwrap();
        assert!(is_legacy);
        assert_eq!(mods.len(), 2);
        assert!(ignored.is_empty());
    }

    #[test]
    fn toml_migration_round_trips() {
        let legacy = "450814997 # CBA_A3\n887302721 # Boat Mod\n\n[ignore]\n463939057 # ACE\n";
        let (mods, ignored, is_legacy) = parse_mod_sources_content(legacy).unwrap();
        assert!(is_legacy);

        let migrated = toml_from_legacy(&mods, &ignored).unwrap();
        assert!(migrated.contains("version = 2"));
        // Ids migrate as clickable Workshop URLs
        assert!(migrated
            .contains("id = \"https://steamcommunity.com/sharedfiles/filedetails/?id=450814997\""));
        assert!(migrated.contains("name = \"CBA_A3\""));
        assert!(migrated.contains("role = \"ignore\""));
        assert!(migrated.contains("enabled = false"));

        // Migrated output parses back with identical contents
        let (mods2, ignored2, is_legacy2) = parse_mod_sources_content(&migrated).unwrap();
        assert!(!is_legacy2);
        assert_eq!(mods2.len(), mods.len());
        assert_eq!(ignored2, ignored);
    }
}
