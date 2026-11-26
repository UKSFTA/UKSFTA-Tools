use super::html::generate_preset;
use super::parse_mod_sources;
use crate::error::UksftaError;
use crate::lock::{load_lock, LockFile};
use std::path::Path;

/// Write every enabled, non-ignored mod in `mod_sources.txt` as an Arma 3
/// launcher preset. A local mod has no Workshop id, so it cannot be exported.
pub fn export_modlist(
    sources_path: &Path,
    lock_path: &Path,
    output: Option<&Path>,
    preset_name: Option<&str>,
    dry_run: bool,
) -> Result<(), UksftaError> {
    let (mods, _ignored) = parse_mod_sources(sources_path)?;
    if mods.is_empty() {
        return Err(UksftaError::Input(
            "No enabled mods in mod_sources.txt".to_string(),
        ));
    }

    let lock = load_lock(lock_path)?;
    let pairs: Vec<(String, String)> = mods
        .iter()
        .map(|m| (m.id.clone(), resolve_name(&m.id, &m.name, &lock)))
        .collect();

    let path = output.unwrap_or(Path::new("modlist.html"));
    let preset = match preset_name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name.to_string(),
        None => default_preset_name(std::env::current_dir().ok().as_deref()),
    };

    if dry_run {
        println!(
            "Dry run: would export {} mod(s) to {}:",
            pairs.len(),
            path.display()
        );
        for (id, name) in &pairs {
            println!("  {} ({})", name, id);
        }
        return Ok(());
    }

    generate_preset(&pairs, path, &preset)
}

/// Use the lock name only when the source holds the `Mod <id>` placeholder.
fn resolve_name(id: &str, source_name: &str, lock: &LockFile) -> String {
    let placeholder = format!("Mod {id}");
    if source_name != placeholder {
        return source_name.to_string();
    }
    match lock.mods.get(id) {
        Some(entry) if !entry.name.is_empty() => entry.name.clone(),
        _ => placeholder,
    }
}

/// The final component of `dir`, or a fixed name when there is none.
fn default_preset_name(dir: Option<&Path>) -> String {
    dir.and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "UKSFTA Modpack".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::parse_modlist_html;
    use std::fs;
    use std::path::PathBuf;

    const SOURCES: &str = r#"version = 2

[[mods]]
id = "450814997"
name = "CBA_A3"

[[mods]]
id = "887302721"
name = "Boat Mod"

[[mods]]
id = "463939057"
name = "Ignored Mod"
role = "ignore"

[[mods]]
id = "33333333"
name = "Disabled Mod"
enabled = false
"#;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("uksfta-export-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_sources(dir: &Path, content: &str) -> PathBuf {
        let path = dir.join("mod_sources.txt");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn export_round_trips_through_parse_modlist_html() {
        let dir = temp_dir("round-trip");
        let sources = write_sources(&dir, SOURCES);
        let lock = dir.join("mods.lock");
        let out = dir.join("modlist.html");

        export_modlist(&sources, &lock, Some(&out), Some("Pack"), false).unwrap();

        let (found, local) = parse_modlist_html(&fs::read_to_string(&out).unwrap());
        assert_eq!(
            found,
            vec![
                ("450814997".to_string(), "CBA_A3".to_string()),
                ("887302721".to_string(), "Boat Mod".to_string()),
            ]
        );
        assert!(local.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_skips_ignored_and_disabled_entries() {
        let dir = temp_dir("filtering");
        let sources = write_sources(&dir, SOURCES);
        let out = dir.join("modlist.html");

        export_modlist(&sources, &dir.join("mods.lock"), Some(&out), None, false).unwrap();

        let html = fs::read_to_string(&out).unwrap();
        assert_eq!(html.matches("data-type=\"ModContainer\"").count(), 2);
        assert!(!html.contains("Ignored Mod"));
        assert!(!html.contains("Disabled Mod"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_escapes_mod_name_and_preset_name() {
        let dir = temp_dir("escaping");
        let sources = write_sources(
            &dir,
            r#"[[mods]]
id = "450814997"
name = "<script>alert(1)</script>"
"#,
        );
        let out = dir.join("modlist.html");

        export_modlist(
            &sources,
            &dir.join("mods.lock"),
            Some(&out),
            Some("<b>Pack</b>"),
            false,
        )
        .unwrap();

        let html = fs::read_to_string(&out).unwrap();
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;&#x2F;script&gt;"));
        assert!(!html.contains("<b>Pack</b>"));
        assert!(html.contains("&lt;b&gt;Pack&lt;&#x2F;b&gt;"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_uses_lock_name_for_placeholder_source_name() {
        let dir = temp_dir("placeholder");
        let sources = write_sources(
            &dir,
            r#"[[mods]]
id = "11111111"
"#,
        );
        let lock = dir.join("mods.lock");
        fs::write(
            &lock,
            r#"{"version":1,"mods":{"11111111":{"name":"Lock Name"}}}"#,
        )
        .unwrap();
        let out = dir.join("modlist.html");

        export_modlist(&sources, &lock, Some(&out), None, false).unwrap();

        let (found, _) = parse_modlist_html(&fs::read_to_string(&out).unwrap());
        assert_eq!(
            found,
            vec![("11111111".to_string(), "Lock Name".to_string())]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_without_lock_uses_source_name() {
        let dir = temp_dir("no-lock");
        let sources = write_sources(
            &dir,
            r#"[[mods]]
id = "11111111"
name = "Source Name"
"#,
        );
        let out = dir.join("modlist.html");

        export_modlist(&sources, &dir.join("mods.lock"), Some(&out), None, false).unwrap();

        let (found, _) = parse_modlist_html(&fs::read_to_string(&out).unwrap());
        assert_eq!(
            found,
            vec![("11111111".to_string(), "Source Name".to_string())]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_errors_when_no_enabled_mods() {
        let dir = temp_dir("empty");
        let sources = write_sources(
            &dir,
            r#"[[mods]]
id = "450814997"
role = "ignore"
"#,
        );
        let err = export_modlist(
            &sources,
            &dir.join("mods.lock"),
            Some(&dir.join("modlist.html")),
            None,
            false,
        )
        .unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn export_dry_run_writes_no_file() {
        let dir = temp_dir("dry-run");
        let sources = write_sources(&dir, SOURCES);
        let out = dir.join("modlist.html");

        export_modlist(&sources, &dir.join("mods.lock"), Some(&out), None, true).unwrap();

        assert!(!out.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_preset_name_uses_final_component_and_falls_back() {
        assert_eq!(
            default_preset_name(Some(Path::new("/tmp/packs/UKSFTA"))),
            "UKSFTA"
        );
        assert_eq!(default_preset_name(None), "UKSFTA Modpack");
        assert_eq!(default_preset_name(Some(Path::new("/"))), "UKSFTA Modpack");
    }
}
