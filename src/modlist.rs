use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::util::{extract_id, extract_tag, toml_escape, workshop_url};

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

/// TOML v2 schema for mod_sources.txt
#[derive(Debug, Serialize, Deserialize)]
struct TomlModSources {
    #[serde(default = "default_sources_version")]
    version: u32,
    #[serde(default)]
    mods: Vec<TomlMod>,
}

fn default_sources_version() -> u32 {
    2
}

#[derive(Debug, Serialize, Deserialize)]
struct TomlMod {
    id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default = "default_role", skip_serializing_if = "is_default_role")]
    role: String,
    #[serde(
        default = "default_enabled",
        skip_serializing_if = "is_default_enabled"
    )]
    enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dependencies: Vec<String>,
}

fn default_role() -> String {
    "mod".to_string()
}

fn default_enabled() -> bool {
    true
}

fn is_default_role(role: &str) -> bool {
    role == "mod"
}

fn is_default_enabled(enabled: &bool) -> bool {
    *enabled
}

/// Parse mod_sources.txt in either TOML (v2) or legacy (v1) format.
/// Returns (mods, ignored ids). Migrates a legacy file to TOML with a .bak
/// backup once it has been read successfully.
pub fn parse_mod_sources(path: &Path) -> (Vec<ModEntry>, Vec<String>) {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("Error: no mod_sources.txt found in this directory");
            eprintln!("Run from a mod repo root containing mod_sources.txt");
            std::process::exit(1);
        }
    };

    let (mods, ignored, is_legacy) = parse_mod_sources_content(&content);
    if is_legacy && !content.trim().is_empty() {
        migrate_mod_sources(path, &mods, &ignored);
    }
    (mods, ignored)
}

/// Parse mod_sources.txt content. Third return value is true when the input
/// was legacy (v1) format, so the caller can migrate it.
pub fn parse_mod_sources_content(content: &str) -> (Vec<ModEntry>, Vec<String>, bool) {
    let looks_toml = content.contains("[[mods]]")
        || content
            .lines()
            .any(|l| l.trim_start().starts_with("version ="));
    if looks_toml {
        match toml::from_str::<TomlModSources>(content) {
            Ok(src) => {
                let mut mods = Vec::new();
                let mut ignored = Vec::new();
                for m in src.mods {
                    // The id field accepts a bare ID or a full Workshop URL
                    let Some(id) = extract_id(&m.id) else {
                        eprintln!("Error: invalid mod id in mod_sources.txt: {}", m.id);
                        std::process::exit(1);
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
                (mods, ignored, false)
            }
            Err(e) => {
                eprintln!("Error: failed to parse TOML mod_sources.txt: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        let (mods, ignored) = parse_legacy_mod_sources(content);
        (mods, ignored, true)
    }
}

/// Legacy v1 parser: one Workshop mod per line, optional `# tag`,
/// optional `[ignore]` / `@ignore` section at the end.
fn parse_legacy_mod_sources(content: &str) -> (Vec<ModEntry>, Vec<String>) {
    let mut mods = Vec::new();
    let mut ignored = Vec::new();
    let mut in_ignore = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            if line.to_lowercase().contains("[ignore]") || line.to_lowercase().contains("@ignore") {
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
        if line.to_lowercase().contains("[ignore]") || line.to_lowercase().contains("@ignore") {
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
pub fn toml_from_legacy(mods: &[ModEntry], ignored: &[String]) -> String {
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
    toml::to_string(&out).expect("Failed to serialise TOML mod sources")
}

/// Rewrite a legacy mod_sources.txt as TOML v2, keeping a .bak backup.
fn migrate_mod_sources(path: &Path, mods: &[ModEntry], ignored: &[String]) {
    let bak_path = path.with_extension("txt.bak");
    if let Err(e) = fs::copy(path, &bak_path) {
        eprintln!(
            "Warning: could not back up {} to {}: {}",
            path.display(),
            bak_path.display(),
            e
        );
        return;
    }
    let toml = toml_from_legacy(mods, ignored);
    match fs::write(path, toml) {
        Ok(_) => println!(
            "Migrated mod_sources.txt to TOML format (backup: {})",
            bak_path.display()
        ),
        Err(e) => eprintln!("Error: could not write migrated {}: {}", path.display(), e),
    }
}

const MODLIST_TEMPLATE_HEADER: &str = "\
<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
<html>\n\
<!--Exported with uksfta: https://github.com/UKSFTA/UKSFTA-Tools-->\n\
  <head>\n\
    <meta name=\"arma:Type\" content=\"preset\" />\n\
    <meta name=\"arma:PresetName\" content=\"UKSFTA Missing Mods\" />\n\
    <meta name=\"generator\" content=\"uksfta\" />\n\
    <title>Arma 3</title>\n\
    <link href=\"https://fonts.googleapis.com/css?family=Roboto\" rel=\"stylesheet\" type=\"text/css\" />\n\
    <style>\n\
body {\n\
\tmargin: 0;\n\
\tpadding: 0;\n\
\tcolor: #fff;\n\
\tbackground: #000;\n\
}\n\
\n\
body, th, td {\n\
\tfont: 95%/1.3 Roboto, Segoe UI, Tahoma, Arial, Helvetica, sans-serif;\n\
}\n\
\n\
td {\n\
    padding: 3px 30px 3px 0;\n\
}\n\
\n\
h1 {\n\
    padding: 20px 20px 0 20px;\n\
    color: white;\n\
    font-weight: 200;\n\
    font-family: segoe ui;\n\
    font-size: 3em;\n\
    margin: 0;\n\
}\n\
\n\
em {\n\
    font-variant: italic;\n\
    color:silver;\n\
}\n\
\n\
.before-list {\n\
    padding: 5px 20px 10px 20px;\n\
}\n\
\n\
.mod-list {\n\
    background: #222222;\n\
    padding: 20px;\n\
}\n\
\n\
.dlc-list {\n\
    background: #222222;\n\
    padding: 20px;\n\
}\n\
\n\
.footer {\n\
    padding: 20px;\n\
    color:gray;\n\
}\n\
\n\
.whups {\n\
    color:gray;\n\
}\n\
\n\
a {\n\
    color: #D18F21;\n\
    text-decoration: underline;\n\
}\n\
\n\
a:hover {\n\
    color:#F1AF41;\n\
    text-decoration: none;\n\
}\n\
\n\
.from-steam {\n\
    color: #449EBD;\n\
}\n\
.from-local {\n\
    color: gray;\n\
}\n\
\n\
</style>\n\
  </head>\n\
  <body>\n\
    <h1>Arma 3  - Preset <strong>UKSFTA Missing Mods</strong></h1>\n\
    <p class=\"before-list\">\n\
      <em>To import this preset, drag this file onto the Launcher window. Or click the MODS tab, then PRESET in the top right, then IMPORT at the bottom, and finally select this file.</em>\n\
    </p>\n\
    <div class=\"mod-list\">\n\
      <table>\n";

const MODLIST_TEMPLATE_FOOTER: &str = "\
      </table>\n\
    </div>\n\
    <div class=\"dlc-list\">\n\
      <table />\n\
    </div>\n\
    <div class=\"footer\">\n\
      <span>Created by uksfta.</span>\n\
    </div>\n\
  </body>\n\
</html>\n";

/// Parse a Workshop page's "Required Items" section.
/// Returns Vec<(id, name)>. Empty if the section is absent.
pub fn parse_required_items(html: &str) -> Vec<(String, String)> {
    let document = scraper::Html::parse_document(html);

    // Steam renders required items in a div with id="RequiredItems"
    // Each item is a link: <a href="...?id=NNN">Name</a>
    let Some(required_section) = document
        .select(&scraper::Selector::parse("#RequiredItems").unwrap())
        .next()
    else {
        return Vec::new();
    };

    let mut deps = Vec::new();
    for link in required_section.select(&scraper::Selector::parse("a").unwrap()) {
        let href = link.value().attr("href").unwrap_or("");
        // Extract id=NNN from the href
        let id = href
            .split("?id=")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let name = link.text().collect::<String>().trim().to_string();
        if !name.is_empty() {
            deps.push((id.to_string(), name));
        }
    }
    deps
}

/// Fetch a Workshop item's page and return its required dependencies as
/// Vec<(id, name)>. Returns empty on network error or if no deps exist.
fn fetch_workshop_dependencies(workshop_id: &str) -> Vec<(String, String)> {
    let url = format!(
        "https://steamcommunity.com/sharedfiles/filedetails/?id={}",
        workshop_id
    );
    // A stalled connection must not hang the CLI indefinitely.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("Failed to build HTTP client");
    let body = match client.get(&url).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "Warning: failed to fetch Workshop page for {}: {}",
                workshop_id, e
            );
            return Vec::new();
        }
    };
    let html = match body.text() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "Warning: failed to read response for {}: {}",
                workshop_id, e
            );
            return Vec::new();
        }
    };
    parse_required_items(&html)
}

/// Resolve transitive dependencies for a list of missing mods.
/// Returns (expanded list, direct deps per fetched mod).
/// The expanded list is missing mods + discovered deps not already known.
/// Deps already present in the workshop cache are skipped entirely.
type DepResolution = (
    Vec<(String, String)>,
    HashMap<String, Vec<(String, String)>>,
);

pub fn resolve_transitive_deps(
    missing: &[(String, String)],
    known_ids: &HashSet<String>,
    cached_ids: &HashSet<String>,
) -> DepResolution {
    let mut result = Vec::new();
    let mut deps_by_mod: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut fetched: HashSet<String> = HashSet::new();
    let mut queue: Vec<(String, String)> = missing.to_vec();

    while let Some((id, name)) = queue.pop() {
        if fetched.contains(&id) {
            continue;
        }
        fetched.insert(id.clone());

        // Missing mods always go in the result
        result.push((id.clone(), name));

        let deps = fetch_workshop_dependencies(&id);
        // Keep only deps we do not already have: not cached, not known
        let missing_deps: Vec<(String, String)> = deps
            .into_iter()
            .filter(|(dep_id, _)| !cached_ids.contains(dep_id) && !known_ids.contains(dep_id))
            .collect();
        // Record for tree display
        deps_by_mod.insert(id.clone(), missing_deps.clone());
        for (dep_id, dep_name) in missing_deps {
            if !fetched.contains(&dep_id) {
                queue.push((dep_id, dep_name));
            }
        }
        // Rate limit: 1 request per second
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    (result, deps_by_mod)
}

/// Print a mod's missing dependency tree with tree-style indentation.
/// Only shows deps that were themselves fetched (i.e. also missing).
pub fn print_dep_tree(
    id: &str,
    deps_by_mod: &HashMap<String, Vec<(String, String)>>,
    prefix: &str,
    visited: &mut HashSet<String>,
) {
    let Some(deps) = deps_by_mod.get(id) else {
        return;
    };
    let missing: Vec<&(String, String)> = deps
        .iter()
        .filter(|(dep_id, _)| deps_by_mod.contains_key(dep_id) && !visited.contains(dep_id))
        .collect();
    for (i, (dep_id, dep_name)) in missing.iter().enumerate() {
        let is_last = i == missing.len() - 1;
        let connector = if is_last { "└─ " } else { "├─ " };
        eprintln!("{}{}{} ({})", prefix, connector, dep_name, dep_id);
        visited.insert(dep_id.clone());
        let child_prefix = if is_last {
            format!("{}   ", prefix)
        } else {
            format!("{}│  ", prefix)
        };
        print_dep_tree(dep_id, deps_by_mod, &child_prefix, visited);
    }
}

/// Build a single mod row for the modlist HTML. Names are HTML-escaped.
pub fn modlist_row_html(id: &str, name: &str) -> String {
    let url = format!(
        "https://steamcommunity.com/sharedfiles/filedetails/?id={}",
        id
    );
    // Mod names come from Steam Workshop pages (attacker-controlled), so
    // escape all HTML metacharacters, not just ampersands.
    let escaped_name = html_escape::encode_safe(name);
    format!(
        "        <tr data-type=\"ModContainer\">\n\
              <td data-type=\"DisplayName\">{}</td>\n\
              <td>\n\
                <span class=\"from-steam\">Steam</span>\n\
              </td>\n\
              <td>\n\
                <a href=\"{}\" data-type=\"Link\">{}</a>\n\
              </td>\n\
            </tr>\n",
        escaped_name, url, url
    )
}

pub fn generate_modlist(missing: &[(String, String)], path: &Path) {
    if missing.is_empty() {
        return;
    }

    let mut html = String::from(MODLIST_TEMPLATE_HEADER);
    for (id, name) in missing {
        html.push_str(&modlist_row_html(id, name));
    }
    html.push_str(MODLIST_TEMPLATE_FOOTER);

    fs::write(path, &html).expect("Failed to write modlist HTML");
    println!("Modlist written to {}", path.display());
}

/// Parse an Arma 3 launcher modlist HTML document.
/// Returns (steam mods as (id, name), local mod names without a Workshop ID).
pub fn parse_modlist_html(content: &str) -> (Vec<(String, String)>, Vec<String>) {
    let document = scraper::Html::parse_document(content);
    let row_selector = scraper::Selector::parse("tr[data-type=\"ModContainer\"]").unwrap();
    let name_selector = scraper::Selector::parse("td[data-type=\"DisplayName\"]").unwrap();
    let link_selector = scraper::Selector::parse("a[data-type=\"Link\"]").unwrap();

    let mut found: Vec<(String, String)> = Vec::new(); // (id, name)
    let mut local_mods: Vec<String> = Vec::new();

    for row in document.select(&row_selector) {
        let name = row
            .select(&name_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();

        let href = row
            .select(&link_selector)
            .next()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or("");

        match extract_id(href) {
            Some(id) => found.push((id, name)),
            None => {
                if !name.is_empty() {
                    local_mods.push(name);
                }
            }
        }
    }
    (found, local_mods)
}

/// Import mods from an Arma 3 launcher modlist HTML file into mod_sources.txt.
/// Appends each Steam mod as "{id} # {name}" (legacy) or a [[mods]] block
/// (TOML). Local mods and duplicates are skipped with a message.
pub fn import_modlist(modlist_file: &Path, dry_run: bool) {
    let content = match fs::read_to_string(modlist_file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: cannot read {}: {}", modlist_file.display(), e);
            std::process::exit(1);
        }
    };

    // Parse <tr data-type="ModContainer"> rows from the launcher export
    let (found, local_mods) = parse_modlist_html(&content);

    if found.is_empty() && local_mods.is_empty() {
        println!(
            "No mods found in {}. Is it an Arma 3 launcher modlist?",
            modlist_file.display()
        );
        return;
    }

    // Load existing IDs to skip duplicates. Handles both legacy and TOML
    // formats via the shared parser.
    let sources_path = Path::new("mod_sources.txt");
    let existing_content = fs::read_to_string(sources_path).unwrap_or_default();
    let is_toml = existing_content.contains("[[mods]]")
        || existing_content
            .lines()
            .any(|l| l.trim_start().starts_with("version ="));
    let (existing_mods, existing_ignored, _) = parse_mod_sources_content(&existing_content);
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
        return;
    }

    if new_mods.is_empty() {
        println!("Nothing new to import.");
        return;
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
        // Insert before [ignore] if present, else append at the end
        let ignore_pos = output
            .lines()
            .position(|l| {
                let l = l.trim().to_lowercase();
                l.contains("[ignore]") || l.contains("@ignore")
            })
            .map(|line_idx| {
                // byte offset of that line's start
                let mut pos = 0usize;
                for (i, line) in output.lines().enumerate() {
                    if i == line_idx {
                        break;
                    }
                    pos += line.len() + 1;
                }
                pos
            });

        let mut additions = String::new();
        for (id, name) in &new_mods {
            // Newlines would break the line-based legacy format.
            // A '#' inside the name is safe: the first '#' is always the
            // separator, so the rest stays part of the name on re-parse.
            let clean_name = name.replace(['\n', '\r'], " ");
            additions.push_str(&format!("{} # {}\n", id, clean_name));
        }

        match ignore_pos {
            Some(pos) => output.insert_str(pos, &additions),
            None => output.push_str(&additions),
        }
    }

    match fs::write(sources_path, output) {
        Ok(_) => println!("Imported {} mod(s) into mod_sources.txt", new_mods.len()),
        Err(e) => {
            eprintln!("Error: cannot write {}: {}", sources_path.display(), e);
            std::process::exit(1);
        }
    }
}
