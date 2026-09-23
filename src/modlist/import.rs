use super::legacy::is_ignore_marker;
use super::migrate::migrate_legacy_if_needed;
use super::parse_html::parse_modlist_html;
use super::parse_mod_sources_content;
use crate::error::UksftaError;
use crate::util::{toml_escape, workshop_url};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Import mods from an Arma 3 launcher modlist HTML file into mod_sources.txt.
/// Appends each Steam mod as "{id} # {name}" (legacy) or a [[mods]] block
/// (TOML). Local mods and duplicates are skipped with a message.
pub fn import_modlist(modlist_file: &Path, dry_run: bool) -> Result<(), UksftaError> {
    let content = fs::read_to_string(modlist_file).map_err(|e| {
        UksftaError::Input(format!("cannot read {}: {}", modlist_file.display(), e))
    })?;

    // Parse <tr data-type="ModContainer"> rows from the launcher export
    let (found, local_mods) = parse_modlist_html(&content);

    if found.is_empty() && local_mods.is_empty() {
        return Err(UksftaError::Input(format!(
            "No mods found in {}. Is it an Arma 3 launcher modlist?",
            modlist_file.display()
        )));
    }

    let sources_path = Path::new("mod_sources.txt");

    // Migrate a legacy file once, before insertion, so the entries appended
    // below match the file's on-disk format.
    migrate_legacy_if_needed(sources_path, dry_run)?;

    // Load existing IDs to skip duplicates. Handles both legacy and TOML
    // formats via the shared parser.
    let existing_content = fs::read_to_string(sources_path).unwrap_or_default();
    let is_toml = existing_content.contains("[[mods]]")
        || existing_content
            .lines()
            .any(|l| l.trim_start().starts_with("version ="));
    let (existing_mods, existing_ignored, _) = parse_mod_sources_content(&existing_content)?;
    let existing: HashSet<String> = existing_mods
        .iter()
        .map(|m| m.id.clone())
        .chain(existing_ignored.iter().cloned())
        .collect();

    let mut new_mods: Vec<(String, String)> = Vec::new();
    for (id, name) in found {
        if existing.contains(&id) {
            println!("Skip {} ({}) — already in mod_sources.txt", name, id);
        } else {
            new_mods.push((id, name));
        }
    }

    if !local_mods.is_empty() {
        println!(
            "Skipped {} local mod(s) with no Workshop ID: {}",
            local_mods.len(),
            local_mods.join(", ")
        );
    }

    if dry_run {
        println!("\nDry run — would add {} mod(s):", new_mods.len());
        for (id, name) in &new_mods {
            println!("  {} # {}", id, name);
        }
        return Ok(());
    }

    if new_mods.is_empty() {
        println!("Nothing new to import.");
        return Ok(());
    }

    let mut output = existing_content;
    if !output.ends_with('\n') {
        output.push('\n');
    }

    if is_toml {
        // Append [[mods]] blocks at the end, separated by a blank line.
        // Ids are written as full Workshop URLs so entries stay clickable.
        if !output.ends_with("\n\n") {
            output.push('\n');
        }
        let mut additions = String::new();
        for (id, name) in &new_mods {
            additions.push_str(&format!("[[mods]]\nid = \"{}\"\n", workshop_url(id)));
            if !name.is_empty() {
                additions.push_str(&format!("name = \"{}\"\n", toml_escape(name)));
            }
            additions.push('\n');
        }
        output.push_str(&additions);
    } else {
        // Insert before [ignore] if present, else append at the end.
        let mut additions = String::new();
        for (id, name) in &new_mods {
            // Newlines would break the line-based legacy format.
            // A '#' inside the name is safe: the first '#' is always the
            // separator, so the rest stays part of the name on re-parse.
            let clean_name = name.replace(['\n', '\r'], " ");
            additions.push_str(&format!("{} # {}\n", id, clean_name));
        }
        output = insert_before_ignore(&output, &additions);
    }

    fs::write(sources_path, output).map_err(UksftaError::Io)?;
    println!("Imported {} mod(s) into mod_sources.txt", new_mods.len());
    Ok(())
}

/// Insert `additions` before the `[ignore]`/`@ignore` marker line. Every
/// byte of `content` is kept, including `\r` in a CRLF file. With no marker
/// the additions are appended at the end.
pub(crate) fn insert_before_ignore(content: &str, additions: &str) -> String {
    let mut offset = 0usize;
    for chunk in content.split_inclusive('\n') {
        if is_ignore_marker(chunk) {
            let mut out = content.to_string();
            out.insert_str(offset, additions);
            return out;
        }
        offset += chunk.len();
    }
    let mut out = content.to_string();
    out.push_str(additions);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_inserts_before_ignore_section() {
        let sources = "450814997 # CBA_A3\n\n[ignore]\n463939057 # ACE\n";
        let out = insert_before_ignore(sources, "1234567890 # O&T Warfighters\n");
        assert!(out.starts_with("450814997 # CBA_A3\n\n1234567890 # O&T Warfighters\n[ignore]"));
    }

    #[test]
    fn import_inserts_before_ignore_section_crlf() {
        let sources = "450814997 # CBA_A3\r\n\r\n[ignore]\r\n463939057 # ACE\r\n";
        let additions = "1234567890 # O&T Warfighters\n";
        let out = insert_before_ignore(sources, additions);
        assert!(out.starts_with("450814997 # CBA_A3\r\n\r\n1234567890"));
        assert!(out.ends_with("[ignore]\r\n463939057 # ACE\r\n"));
        assert_eq!(out.len(), sources.len() + additions.len());
    }

    // Mirrors the assertion made through the module re-export path.
    #[test]
    fn import_inserts_before_ignore_section_preserves_entries() {
        let sources = "450814997 # CBA_A3\n\n[ignore]\n463939057 # ACE\n";
        let out = insert_before_ignore(sources, "1234567890 # O&T Warfighters\n");
        assert!(out.starts_with("450814997 # CBA_A3\n\n1234567890 # O&T Warfighters\n[ignore]"));
    }
}
