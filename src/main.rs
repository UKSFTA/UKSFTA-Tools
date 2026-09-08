use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const STEAM_APP_ID: &str = "107410";

// --- VDF Metadata ---

#[derive(Debug, Clone, Default)]
struct WorkshopItem {
    size: u64,
    time_updated: u64,
}

fn parse_acf(content: &str) -> HashMap<String, WorkshopItem> {
    let mut items = HashMap::new();
    let mut current_id: Option<String> = None;
    let mut current_item = WorkshopItem::default();
    let mut in_installed = false;
    let mut brace_depth = 0;

    for line in content.lines() {
        let line = line.trim();

        if line.contains("\"WorkshopItemsInstalled\"") {
            in_installed = true;
            brace_depth = 0;
            continue;
        }

        if in_installed {
            if line.starts_with('{') {
                brace_depth += 1;
                continue;
            }
            if line.starts_with('}') {
                brace_depth -= 1;
                if brace_depth == 0 {
                    // Save last item if any
                    if let Some(id) = &current_id {
                        items.insert(id.clone(), current_item.clone());
                        current_id = None;
                        current_item = WorkshopItem::default();
                    }
                    in_installed = false;
                    continue;
                }
                continue;
            }

            // At depth 1, keys are published IDs
            if brace_depth == 1 {
                if let Some(val) = extract_quoted(line) {
                    if val.chars().all(|c| c.is_ascii_digit()) {
                        // Save previous item
                        if let Some(id) = &current_id {
                            items.insert(id.clone(), current_item.clone());
                        }
                        current_id = Some(val);
                        current_item = WorkshopItem::default();
                    }
                }
            }

            // At depth 2, we're inside an item's fields
            if brace_depth == 2 {
                let tokens = extract_quoted_tokens(line);
                if tokens.len() >= 2 {
                    if tokens[0] == "size" {
                        current_item.size = tokens[1].parse().unwrap_or(0);
                    } else if tokens[0] == "timeupdated" {
                        current_item.time_updated = tokens[1].parse().unwrap_or(0);
                    }
                }
            }
        }
    }

    // Save last item
    if let Some(id) = &current_id {
        items.insert(id.clone(), current_item);
    }

    items
}

fn extract_quoted(line: &str) -> Option<String> {
    let chars = line.chars();
    let mut in_quote = false;
    let mut value = String::new();

    for c in chars {
        if c == '"' {
            if in_quote {
                return Some(value);
            }
            in_quote = true;
        } else if in_quote {
            value.push(c);
        }
    }
    None
}

fn extract_quoted_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars = line.chars();
    let mut in_quote = false;
    let mut value = String::new();

    for c in chars {
        if c == '"' {
            if in_quote {
                tokens.push(std::mem::take(&mut value));
                in_quote = false;
            } else {
                in_quote = true;
            }
        } else if in_quote {
            value.push(c);
        }
    }
    tokens
}

// --- mod.cpp / meta.cpp parsing ---

#[derive(Debug, Clone, Default)]
struct ModCpp {
    name: String,
    author: String,
    publishedid: Option<String>,
}

fn parse_mod_cpp(path: &Path) -> ModCpp {
    let mut result = ModCpp::default();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = extract_quoted(line) {
                if line.starts_with("name") {
                    result.name = val;
                } else if line.starts_with("author") {
                    result.author = val;
                }
            }
        }
    }
    result
}

fn parse_meta_cpp(path: &Path) -> ModCpp {
    let mut result = ModCpp::default();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = extract_quoted(line) {
                if line.starts_with("publishedid") {
                    result.publishedid = Some(val);
                } else if line.starts_with("name") {
                    result.name = val;
                }
            }
        }
    }
    result
}

fn get_mod_metadata(mod_dir: &Path) -> ModCpp {
    let mut meta = ModCpp::default();

    // mod.cpp has presentation metadata
    let mod_cpp = mod_dir.join("mod.cpp");
    if mod_cpp.exists() {
        meta = parse_mod_cpp(&mod_cpp);
    }

    // meta.cpp has Workshop ID
    let meta_cpp = mod_dir.join("meta.cpp");
    if meta_cpp.exists() {
        let workshop_meta = parse_meta_cpp(&meta_cpp);
        if let Some(id) = workshop_meta.publishedid {
            meta.publishedid = Some(id);
        }
        if meta.name.is_empty() && !workshop_meta.name.is_empty() {
            meta.name = workshop_meta.name;
        }
    }

    meta
}

#[derive(Parser)]
#[command(name = "uksfta", about = "UKSFTA modpack manager")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Sync mods from Workshop cache into addons/
    Sync {
        /// Show what would change without doing it
        #[arg(long)]
        dry_run: bool,
        /// Skip dependency resolution (offline mode)
        #[arg(long)]
        offline: bool,
        /// Generate an Arma 3 launcher modlist for missing mods
        #[arg(long)]
        modlist: bool,
        /// Output path for the modlist HTML (default: missing-mods.html)
        #[arg(long, default_value = "missing-mods.html")]
        modlist_path: PathBuf,
        /// Resolve missing mods' dependencies from Workshop pages
        #[arg(long)]
        resolve_deps: bool,
    },
    /// Show which PBOs came from which Workshop mod
    Identify,
    /// Verify all PBOs are present
    Verify,
    /// Audit each mod's PBOs against addons/ (present or missing)
    Audit {
        /// Show only missing PBOs and per-mod counts
        #[arg(long)]
        missing_only: bool,
    },
    /// Check for updates available in Workshop cache
    Updates,
    /// Import mods from an Arma 3 launcher modlist HTML file
    Import {
        /// Path to the modlist HTML file
        modlist_file: PathBuf,
        /// Show what would be imported without modifying mod_sources.txt
        #[arg(long)]
        dry_run: bool,
    },
}

// --- Mod list parsing ---

#[derive(Debug, Clone)]
struct ModEntry {
    id: String,
    name: String,
    tags: Vec<String>,
    role: String,
    enabled: bool,
    dependencies: Vec<String>,
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
fn parse_mod_sources(path: &Path) -> (Vec<ModEntry>, Vec<String>) {
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
fn parse_mod_sources_content(content: &str) -> (Vec<ModEntry>, Vec<String>, bool) {
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

/// Build the Steam Workshop page URL for a mod ID.
fn workshop_url(id: &str) -> String {
    format!(
        "https://steamcommunity.com/sharedfiles/filedetails/?id={}",
        id
    )
}

/// Serialise legacy data into the TOML v2 format.
/// Ids are written as full Workshop URLs so entries stay clickable.
fn toml_from_legacy(mods: &[ModEntry], ignored: &[String]) -> String {
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

fn extract_id(line: &str) -> Option<String> {
    // Match 8+ digit IDs in URLs or bare
    for word in line.split_whitespace() {
        if let Some(pos) = word.find("id=") {
            let rest = &word[pos + 3..];
            if let Some(end) = rest.find(|c: char| !c.is_ascii_digit()) {
                let id = &rest[..end];
                if id.len() >= 8 {
                    return Some(id.to_string());
                }
            } else if rest.len() >= 8 {
                return Some(rest.to_string());
            }
        }
        // Bare ID
        if word.chars().all(|c| c.is_ascii_digit()) && word.len() >= 8 {
            return Some(word.to_string());
        }
    }
    None
}

fn extract_tag(line: &str) -> Option<String> {
    if let Some(pos) = line.find('#') {
        let tag = line[pos + 1..].trim().to_string();
        if !tag.is_empty() {
            return Some(tag);
        }
    }
    None
}

// --- Steam library discovery ---
//
// We do not guess where Steam libraries live. Steam tells us:
//   - Windows/WSL: HKCU\Software\Valve\Steam -> SteamPath (what Steam itself reads)
//   - Linux:       ~/.steam/steam (a symlink Steam maintains to its real root)
// From that root, libraryfolders.vdf lists every library Steam owns.

fn steam_registry_path() -> Option<PathBuf> {
    // `reg.exe` works on native Windows and inside WSL (Windows interop),
    // so one code path covers both.
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().find_map(|line| {
        // Registry output: "    SteamPath    REG_SZ    C:\Program Files (x86)\Steam"
        let rest = line.split_once("REG_SZ")?.1.trim();
        if rest.is_empty() {
            None
        } else {
            Some(PathBuf::from(rest))
        }
    })
}

fn steam_root() -> Option<PathBuf> {
    #[cfg(any(target_os = "windows", target_os = "linux"))]
    {
        // Windows registry path (also reachable from WSL via interop).
        if let Some(p) = steam_registry_path() {
            // In WSL, a Windows path like C:\... maps to /mnt/c/...
            #[cfg(target_os = "linux")]
            if let Some(mapped) = wsl_map_windows_path(&p) {
                return Some(mapped);
            }
            #[cfg(target_os = "windows")]
            return Some(p);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir()?;
        // Steam maintains ~/.steam/steam as a symlink to its real root.
        let candidates = [home.join(".steam/steam"), home.join(".local/share/Steam")];
        for c in candidates {
            if c.join("steamapps").exists() {
                return Some(c);
            }
        }
    }

    None
}

/// Maps a Windows path (C:\foo) to its WSL mount (/mnt/c/foo).
#[cfg(target_os = "linux")]
fn wsl_map_windows_path(win: &Path) -> Option<PathBuf> {
    let s = win.to_str()?;
    // Expect "<drive>:\\<rest>"
    let drive = s.as_bytes().first()?.to_ascii_lowercase() as char;
    if !drive.is_ascii_alphabetic() {
        return None;
    }
    let rest = s.get(2..)?.trim_start_matches('\\');
    let mut mapped = PathBuf::from("/mnt");
    mapped.push(drive.to_string());
    mapped.push(rest.replace('\\', "/"));
    Some(mapped)
}

fn find_steam_library_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();

    if let Some(root) = steam_root() {
        // Steam's authoritative library list.
        for vdf_path in [
            root.join("config/libraryfolders.vdf"),
            root.join("steamapps/libraryfolders.vdf"), // fallback
        ] {
            if let Ok(content) = fs::read_to_string(&vdf_path) {
                parse_libraryfolders_vdf(&content, &mut folders);
            }
        }

        // The root's own steamapps dir.
        let steamapps = root.join("steamapps");
        if steamapps.exists() && !folders.contains(&steamapps) {
            folders.push(steamapps);
        }
    }

    folders
}

fn parse_libraryfolders_vdf(content: &str, folders: &mut Vec<PathBuf>) {
    for line in content.lines() {
        let line = line.trim();
        // VDF lines: "path" "C:\SteamLibrary"  -> value is the SECOND quoted token
        let tokens = extract_quoted_tokens(line);
        if tokens.len() >= 2 && tokens[0] == "path" {
            let steamapps = PathBuf::from(&tokens[1]).join("steamapps");
            if !folders.contains(&steamapps) {
                folders.push(steamapps);
            }
        }
    }
}

fn find_all_workshop_caches() -> Vec<PathBuf> {
    let folders = find_steam_library_folders();
    let mut caches = Vec::new();
    for folder in folders {
        let cache = folder.join("workshop/content").join(STEAM_APP_ID);
        if cache.exists() {
            caches.push(cache);
        }
    }
    caches
}

// --- Lock file ---

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct LockFile {
    #[serde(default = "default_lock_version")]
    version: u32,
    mods: HashMap<String, ModLockEntry>,
}

fn default_lock_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct ModLockEntry {
    files: Vec<String>,
    name: String,
    #[serde(default)]
    tags: Vec<String>,
    dependencies: Vec<Dependency>,
    updated: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
struct Dependency {
    id: String,
    name: String,
}

fn load_lock(path: &Path) -> LockFile {
    if path.exists() {
        let content = fs::read_to_string(path).expect("Failed to read mods.lock");
        serde_json::from_str(&content).unwrap_or(LockFile {
            version: default_lock_version(),
            mods: HashMap::new(),
        })
    } else {
        LockFile {
            version: default_lock_version(),
            mods: HashMap::new(),
        }
    }
}

fn save_lock(path: &Path, lock: &LockFile) {
    let json = serde_json::to_string_pretty(lock).expect("Failed to serialize lock");
    fs::write(path, json).expect("Failed to write mods.lock");
}

// --- PBO sync ---

fn find_pbos(dir: &Path) -> Vec<PathBuf> {
    let mut pbos = Vec::new();
    if !dir.exists() {
        return pbos;
    }
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry.expect("Failed to read directory");
        if entry.file_type().is_file() {
            if let Some(ext) = entry.path().extension() {
                if ext.to_string_lossy().to_lowercase() == "pbo" {
                    pbos.push(entry.path().to_path_buf());
                }
            }
        }
    }
    pbos
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
fn parse_required_items(html: &str) -> Vec<(String, String)> {
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
    let body = match reqwest::blocking::get(&url) {
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

fn resolve_transitive_deps(
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
fn print_dep_tree(
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
fn modlist_row_html(id: &str, name: &str) -> String {
    let url = format!(
        "https://steamcommunity.com/sharedfiles/filedetails/?id={}",
        id
    );
    let escaped_name = name.replace('&', "&amp;");
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

fn generate_modlist(missing: &[(String, String)], path: &Path) {
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

fn sync_mods(
    mods: &[ModEntry],
    ignored: &[String],
    dry_run: bool,
    _offline: bool,
    modlist: bool,
    modlist_path: &Path,
    resolve_deps: bool,
) {
    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        eprintln!("Workshop cache not found. Is Steam installed?");
        return;
    }
    let addons_dir = Path::new("addons");
    let lock_path = Path::new("mods.lock");

    let lock = load_lock(lock_path);

    // Load VDF metadata for timestamps from all caches
    let mut workshop_items = HashMap::new();
    for cache in &caches {
        // cache = .../workshop/content/107410
        // ACF is at .../workshop/appworkshop_107410.acf
        if let Some(workshop_dir) = cache.parent().and_then(|p| p.parent()) {
            let acf = workshop_dir.join(format!("appworkshop_{}.acf", STEAM_APP_ID));
            if acf.exists() {
                if let Ok(content) = fs::read_to_string(&acf) {
                    workshop_items.extend(parse_acf(&content));
                }
            }
        }
    }

    // Classify each requested mod: added / updated / unchanged / missing from cache
    let mut planned: Vec<(ModLockEntry, String, String, Vec<PathBuf>)> = Vec::new(); // (entry, id, status, source PBO paths)
    let mut missing_from_cache = Vec::new(); // (id, name)

    for entry in mods {
        // Find the mod in any cache
        let mut mod_path = None;
        for cache in &caches {
            let path = cache.join(&entry.id);
            if path.exists() {
                mod_path = Some(path);
                break;
            }
        }

        let mod_path = match mod_path {
            Some(p) => p,
            None => {
                missing_from_cache.push((entry.id.clone(), entry.name.clone()));
                continue;
            }
        };

        // Get metadata from mod.cpp/meta.cpp if available
        let mod_meta = get_mod_metadata(&mod_path);
        let display_name = if mod_meta.name.is_empty() {
            entry.name.clone()
        } else {
            mod_meta.name
        };

        let pbos = find_pbos(&mod_path);
        if pbos.is_empty() {
            missing_from_cache.push((entry.id.clone(), entry.name.clone()));
            continue;
        }

        // Timestamp from ACF (Workshop's last update)
        let updated = workshop_items
            .get(&entry.id)
            .map(|item| item.time_updated.to_string())
            .unwrap_or_else(|| "0".to_string());

        let files: Vec<String> = pbos
            .iter()
            .map(|p| {
                addons_dir
                    .join(p.file_name().unwrap())
                    .to_string_lossy()
                    .to_string()
            })
            .collect();

        // Classify against lock
        let status = match lock.mods.get(&entry.id) {
            None => "added",
            Some(locked) => {
                let files_exist = locked.files.iter().all(|f| Path::new(f).exists());
                if locked.updated != updated || !files_exist {
                    "updated"
                } else {
                    "unchanged"
                }
            }
        };

        // Resolve declared dependency names from the mods list where possible
        let dependencies: Vec<Dependency> = entry
            .dependencies
            .iter()
            .map(|dep_id| Dependency {
                id: dep_id.clone(),
                name: mods
                    .iter()
                    .find(|m| &m.id == dep_id)
                    .map(|m| m.name.clone())
                    .unwrap_or_default(),
            })
            .collect();

        planned.push((
            ModLockEntry {
                files,
                name: display_name,
                tags: entry.tags.clone(),
                dependencies,
                updated,
            },
            entry.id.clone(),
            status.to_string(),
            pbos,
        ));
    }

    // Removed: in lock but not in current mod list (and not ignored)
    let mut removed: Vec<(String, String, Vec<String>)> = Vec::new(); // (id, name, files)
    for (id, old_entry) in &lock.mods {
        if !mods.iter().any(|m| &m.id == id) && !ignored.contains(id) {
            removed.push((id.clone(), old_entry.name.clone(), old_entry.files.clone()));
        }
    }

    let added = planned.iter().filter(|(_, _, s, _)| s == "added").count();
    let updated = planned.iter().filter(|(_, _, s, _)| s == "updated").count();
    let unchanged = planned
        .iter()
        .filter(|(_, _, s, _)| s == "unchanged")
        .count();

    // Print the diff preview
    if dry_run {
        println!("--- Diff preview (dry-run) ---");
        for (entry, id, status, _sources) in &planned {
            match status.as_str() {
                "added" => {
                    println!("  [ADD]     {} ({})", entry.name, id);
                    for f in &entry.files {
                        println!(
                            "    + {}",
                            Path::new(f).file_name().unwrap().to_string_lossy()
                        );
                    }
                }
                "updated" => {
                    println!("  [UPDATE]  {} ({})", entry.name, id);
                    for f in &entry.files {
                        println!(
                            "    ~ {}",
                            Path::new(f).file_name().unwrap().to_string_lossy()
                        );
                    }
                }
                _ => println!(
                    "  [OK]      {} ({}) — {} PBOs present",
                    entry.name,
                    id,
                    entry.files.len()
                ),
            }
        }
        for (id, name, files) in &removed {
            println!("  [REMOVE]  {} ({})", name, id);
            for f in files {
                println!(
                    "    - {}",
                    Path::new(f).file_name().unwrap().to_string_lossy()
                );
            }
        }
        for (id, name) in &missing_from_cache {
            println!("  [MISSING] {} ({}) not found in Workshop cache", name, id);
            println!(
                "            https://steamcommunity.com/sharedfiles/filedetails/?id={}",
                id
            );
        }
        if modlist && !missing_from_cache.is_empty() {
            println!("\nModlist would be written to {}", modlist_path.display());
        }
        println!(
            "\nSummary: {} added, {} updated, {} unchanged, {} removed",
            added,
            updated,
            unchanged,
            removed.len()
        );
        return;
    }

    // Apply changes
    let mut new_lock = LockFile {
        version: default_lock_version(),
        mods: HashMap::new(),
    };

    for (entry, id, status, sources) in &planned {
        if status == "unchanged" {
            // Keep the existing lock entry as-is
            new_lock.mods.insert(id.clone(), lock.mods[id].clone());
            continue;
        }

        fs::create_dir_all(addons_dir).expect("Failed to create addons directory");
        // files[i] is the dest path, sources[i] is the real source path
        for (dest, src) in entry.files.iter().zip(sources.iter()) {
            fs::copy(src, dest).expect("Failed to copy PBO");
        }
        new_lock.mods.insert(id.clone(), entry.clone());
        println!(
            "{}: {} ({})",
            if status == "added" {
                "Added"
            } else {
                "Updated"
            },
            entry.name,
            id
        );
    }

    // Remove PBOs for mods no longer in the list
    for (id, name, files) in &removed {
        for file in files {
            if Path::new(file).exists() {
                fs::remove_file(file).expect("Failed to remove PBO");
            }
        }
        println!("Removed: {} ({})", name, id);
    }

    // Preserve lock entries for mods still in the list but missing from
    // the Workshop cache. They are wanted but not currently syncable, so
    // their previous file records must not be dropped.
    for (id, _) in &missing_from_cache {
        if let Some(entry) = lock.mods.get(id) {
            new_lock.mods.insert(id.clone(), entry.clone());
        }
    }

    // Only write the lock when its content actually changed. This avoids
    // touching the file (and dirtying the working tree) when nothing was
    // synced or removed.
    let changed = new_lock.mods != lock.mods;
    if changed {
        save_lock(lock_path, &new_lock);
    }

    // Resolve dependencies once (only when requested) so warnings and
    // modlist both benefit without double-fetching Workshop pages.
    let (mods_for_list, deps_by_mod) = if resolve_deps {
        let known_ids: std::collections::HashSet<String> =
            mods.iter().map(|m| m.id.clone()).collect();
        let cached_ids: std::collections::HashSet<String> = caches
            .iter()
            .flat_map(|cache| {
                fs::read_dir(cache)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .filter_map(|e| e.file_name().into_string().ok())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .collect();
        resolve_transitive_deps(&missing_from_cache, &known_ids, &cached_ids)
    } else {
        (missing_from_cache.clone(), HashMap::new())
    };

    for (id, name) in &missing_from_cache {
        eprintln!("Warning: {} ({}) not found in Workshop cache", name, id);
        if resolve_deps {
            let mut visited = HashSet::new();
            visited.insert(id.clone());
            print_dep_tree(id, &deps_by_mod, "         ", &mut visited);
        }
        eprintln!(
            "         https://steamcommunity.com/sharedfiles/filedetails/?id={}",
            id
        );
    }

    if !missing_from_cache.is_empty() {
        println!(
            "\nSubscribe to the {} missing mods in Steam:",
            missing_from_cache.len()
        );
        for (id, _) in &missing_from_cache {
            println!("steam://url/CommunityFilePage/{}", id);
        }

        if modlist {
            generate_modlist(&mods_for_list, modlist_path);
        }
    } else if modlist {
        println!("No missing mods — modlist not generated.");
    }

    println!(
        "\nSummary: {} added, {} updated, {} unchanged, {} removed",
        added,
        updated,
        unchanged,
        removed.len()
    );
}

fn identify() {
    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        eprintln!("Workshop cache not found");
        return;
    }

    let addons_dir = Path::new("addons");
    if !addons_dir.exists() {
        eprintln!("No addons/ directory");
        return;
    }

    // Build map of PBO -> workshop ID from all caches
    let mut pbo_map: HashMap<String, String> = HashMap::new();
    for cache in &caches {
        if let Ok(entries) = fs::read_dir(cache) {
            for entry in entries.flatten() {
                let mod_path = entry.path();
                if mod_path.is_dir() {
                    let id = mod_path.file_name().unwrap().to_string_lossy().to_string();
                    for pbo in find_pbos(&mod_path) {
                        if let Some(name) = pbo.file_name() {
                            pbo_map.insert(name.to_string_lossy().to_string(), id.clone());
                        }
                    }
                }
            }
        }
    }

    println!("PBO Origins:");
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .map(|e| e == "pbo")
                .unwrap_or(false)
            {
                let name = entry.file_name().to_string_lossy().to_string();
                let origin = pbo_map.get(&name).map(|s| s.as_str()).unwrap_or("Unknown");
                println!("  {} -> Workshop {}", name, origin);
            }
        }
    }
}

fn verify() {
    let lock_path = Path::new("mods.lock");
    if !lock_path.exists() {
        eprintln!("No mods.lock found. Run sync first.");
        return;
    }

    let lock = load_lock(lock_path);
    let mut missing = 0;

    for (id, entry) in &lock.mods {
        for file in &entry.files {
            if !Path::new(file).exists() {
                eprintln!("Missing: {} ({}) -> {}", entry.name, id, file);
                missing += 1;
            }
        }
    }

    if missing > 0 {
        eprintln!("\n{} PBOs missing", missing);
        std::process::exit(1);
    } else {
        println!(
            "All {} mods verified ({} PBOs)",
            lock.mods.len(),
            lock.mods.values().map(|m| m.files.len()).sum::<usize>()
        );
    }
}

fn audit(missing_only: bool) {
    let (mods, _ignored) = parse_mod_sources(Path::new("mod_sources.txt"));
    if mods.is_empty() {
        eprintln!("No mods found in mod_sources.txt");
        return;
    }

    let mod_count = mods.len();

    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        eprintln!("Workshop cache not found. Is Steam installed?");
        return;
    }

    let addons_dir = Path::new("addons");

    // Build a set of every PBO filename currently in addons/ (recursive)
    let mut present_pbos: HashMap<String, ()> = HashMap::new();
    for pbo in find_pbos(addons_dir) {
        if let Some(name) = pbo.file_name().map(|n| n.to_string_lossy().to_string()) {
            present_pbos.insert(name, ());
        }
    }

    // Collect every expected PBO name across all mods, for orphan detection
    let mut all_expected: HashMap<String, String> = HashMap::new(); // pbo name -> mod name
    let mut not_in_cache = 0;
    let mut total_missing = 0;
    let mut total_expected = 0;
    let mut missing_list: Vec<String> = Vec::new();

    println!("--- Mod audit ---");

    for entry in &mods {
        // Find this mod in any cache
        let mut mod_path = None;
        for cache in &caches {
            let path = cache.join(&entry.id);
            if path.exists() {
                mod_path = Some(path);
                break;
            }
        }

        let mod_path = match mod_path {
            Some(p) => p,
            None => {
                println!(
                    "  [NOT IN CACHE] {} ({}) — cannot enumerate PBOs",
                    entry.name, entry.id
                );
                not_in_cache += 1;
                continue;
            }
        };

        // Expected PBOs from the cache, by filename
        let expected = find_pbos(&mod_path);
        if expected.is_empty() {
            println!(
                "  [EMPTY]   {} ({}) — no PBOs in cache",
                entry.name, entry.id
            );
            continue;
        }

        let expected_names: Vec<String> = expected
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .collect();
        total_expected += expected_names.len();

        let present_count = expected_names
            .iter()
            .filter(|n| present_pbos.contains_key(*n))
            .count();
        let missing: Vec<&String> = expected_names
            .iter()
            .filter(|n| !present_pbos.contains_key(*n))
            .collect();

        for name in &expected_names {
            all_expected
                .entry(name.clone())
                .or_insert_with(|| entry.name.clone());
        }

        let status = if missing.is_empty() {
            "OK"
        } else {
            "INCOMPLETE"
        };
        if !missing_only || !missing.is_empty() {
            println!(
                "  [{}] {} ({}) — {}/{} PBOs",
                status,
                entry.name,
                entry.id,
                present_count,
                expected_names.len()
            );
        }

        if !missing_only {
            for name in &expected_names {
                if present_pbos.contains_key(name) {
                    println!("      present: {}", name);
                } else {
                    println!("      MISSING: {}", name);
                }
            }
        } else {
            for name in &missing {
                println!("      MISSING: {}", name);
            }
        }

        for name in &missing {
            missing_list.push(format!("{} -> {}", name, entry.name));
        }
        total_missing += missing.len();
    }

    // Orphan detection: PBOs in addons/ that belong to no expected mod
    let orphans: Vec<&String> = present_pbos
        .keys()
        .filter(|name| !all_expected.contains_key(*name))
        .collect();
    if !orphans.is_empty() {
        println!("\n  [ORPHANS] PBOs in addons/ not from any listed mod:");
        for name in &orphans {
            println!("      orphan: {}", name);
        }
    }

    let pct = total_expected
        .checked_sub(total_missing)
        .and_then(|n| n.checked_mul(100))
        .and_then(|n| n.checked_div(total_expected))
        .unwrap_or(100);

    println!(
        "\nSummary: {}/{} PBOs present ({}%) across {} mods; {} not in cache",
        total_expected - total_missing,
        total_expected,
        pct,
        mod_count,
        not_in_cache
    );

    if !missing_list.is_empty() {
        println!("\nMissing PBOs:");
        for m in &missing_list {
            println!("  {}", m);
        }
        std::process::exit(1);
    }
}

fn check_updates() {
    let lock_path = Path::new("mods.lock");
    if !lock_path.exists() {
        eprintln!("No mods.lock found. Run sync first.");
        return;
    }

    let lock = load_lock(lock_path);

    // Load ACF from all Steam libraries
    let mut workshop_items = HashMap::new();
    let folders = find_steam_library_folders();
    for folder in &folders {
        let acf = folder
            .join("workshop")
            .join(format!("appworkshop_{}.acf", STEAM_APP_ID));
        if acf.exists() {
            if let Ok(content) = fs::read_to_string(&acf) {
                workshop_items.extend(parse_acf(&content));
            }
        }
    }

    if workshop_items.is_empty() {
        eprintln!("Workshop cache not found");
        return;
    }

    println!("Update Check:");
    let mut updatable = 0;

    for (id, entry) in &lock.mods {
        if let Some(item) = workshop_items.get(id) {
            let lock_time: u64 = entry.updated.parse().unwrap_or(0);
            if item.time_updated > lock_time {
                println!(
                    "  {} ({}) - Workshop updated: {} > locked: {}",
                    entry.name, id, item.time_updated, lock_time
                );
                updatable += 1;
            }
        }
    }

    if updatable == 0 {
        println!("  All mods up to date");
    } else {
        println!("\n{} mods have updates available", updatable);
        println!("Run 'uksfta sync' to apply updates");
    }
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Sync {
            dry_run,
            offline,
            modlist,
            modlist_path,
            resolve_deps,
        } => {
            let (mods, ignored) = parse_mod_sources(Path::new("mod_sources.txt"));
            if mods.is_empty() {
                eprintln!("No mods found in mod_sources.txt");
                return;
            }
            println!("Found {} mods, {} ignored", mods.len(), ignored.len());
            sync_mods(
                &mods,
                &ignored,
                dry_run,
                offline,
                modlist,
                &modlist_path,
                resolve_deps,
            );
        }
        Commands::Identify => identify(),
        Commands::Verify => verify(),
        Commands::Audit { missing_only } => audit(missing_only),
        Commands::Updates => check_updates(),
        Commands::Import {
            modlist_file,
            dry_run,
        } => import_modlist(&modlist_file, dry_run),
    }
}

/// Parse an Arma 3 launcher modlist HTML document.
/// Returns (steam mods as (id, name), local mod names without a Workshop ID).
fn parse_modlist_html(content: &str) -> (Vec<(String, String)>, Vec<String>) {
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
fn import_modlist(modlist_file: &Path, dry_run: bool) {
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
                additions.push_str(&format!("name = \"{}\"\n", name));
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
            additions.push_str(&format!("{} # {}\n", id, name));
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

#[cfg(test)]
mod tests {
    use super::*;
    // --- extract_id ---
    #[test]
    fn extract_id_sharedfiles_url() {
        assert_eq!(
            extract_id("https://steamcommunity.com/sharedfiles/filedetails/?id=450814997").unwrap(),
            "450814997"
        );
    }
    #[test]
    fn extract_id_workshop_url() {
        // Current Steam pages link deps via /workshop/filedetails/
        assert_eq!(
            extract_id("https://steamcommunity.com/workshop/filedetails/?id=2262006564").unwrap(),
            "2262006564"
        );
    }
    #[test]
    fn extract_id_bare() {
        assert_eq!(extract_id("450814997").unwrap(), "450814997");
    }
    #[test]
    fn extract_id_with_name_comment() {
        assert_eq!(extract_id("450814997 # CBA_A3").unwrap(), "450814997");
    }
    #[test]
    fn extract_id_rejects_short_numbers() {
        // Steam IDs are 8+ digits; 7-digit numbers must not match
        assert_eq!(extract_id("1234567"), None);
        assert_eq!(extract_id("id=1234567"), None);
    }
    #[test]
    fn extract_id_rejects_non_id() {
        assert_eq!(extract_id("no id here"), None);
        assert_eq!(extract_id(""), None);
    }
    // --- parse_modlist_html ---
    const MODLIST_SAMPLE: &str = r#"<html><body><div class="mod-list"><table>
<tr data-type="ModContainer">
<td data-type="DisplayName">CBA_A3</td>
<td><span class="from-steam">Steam</span></td>
<td><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=450814997" data-type="Link">URL</a></td>
</tr>
<tr data-type="ModContainer">
<td data-type="DisplayName">O&amp;T Warfighters</td>
<td><span class="from-steam">Steam</span></td>
<td><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=1234567890" data-type="Link">URL</a></td>
</tr>
<tr data-type="ModContainer">
<td data-type="DisplayName">Local Custom Mod</td>
<td><span class="from-local">Local</span></td>
<td></td>
</tr>
</table></div></body></html>"#;
    #[test]
    fn parse_modlist_html_extracts_steam_and_local() {
        let (found, local) = parse_modlist_html(MODLIST_SAMPLE);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0], ("450814997".to_string(), "CBA_A3".to_string()));
        // HTML entities are decoded by the parser
        assert_eq!(
            found[1],
            ("1234567890".to_string(), "O&T Warfighters".to_string())
        );
        assert_eq!(local, vec!["Local Custom Mod".to_string()]);
    }
    #[test]
    fn parse_modlist_html_empty() {
        let (found, local) = parse_modlist_html("<html><body></body></html>");
        assert!(found.is_empty());
        assert!(local.is_empty());
    }
    #[test]
    fn parse_modlist_html_not_a_modlist() {
        let (found, local) = parse_modlist_html("this is not html");
        assert!(found.is_empty());
        assert!(local.is_empty());
    }
    // --- parse_required_items ---
    #[test]
    fn parse_required_items_real_structure() {
        // Mirrors the current Steam HTML: links wrap a div.requiredItem
        let html = r#"<div id="rightContents">
<div class="requiredItemsContainer" id="RequiredItems">
<a href="https://steamcommunity.com/workshop/filedetails/?id=2262006564" target="_blank" data-subscribed="0">
<div class="requiredItem">cTab 1erGTD</div>
</a>
<a href="https://steamcommunity.com/workshop/filedetails/?id=2853828143" target="_blank" data-subscribed="0">
<div class="requiredItem">Better CAS Environment (BCE)</div>
</a>
</div>
</div>"#;
        let deps = parse_required_items(html);
        assert_eq!(
            deps,
            vec![
                ("2262006564".to_string(), "cTab 1erGTD".to_string()),
                (
                    "2853828143".to_string(),
                    "Better CAS Environment (BCE)".to_string()
                ),
            ]
        );
    }
    #[test]
    fn parse_required_items_no_section() {
        assert!(parse_required_items("<html><body>no deps</body></html>").is_empty());
    }
    // --- modlist_row_html ---
    #[test]
    fn modlist_row_html_escapes_ampersand() {
        let row = modlist_row_html("1234567890", "O&T Warfighters");
        assert!(row.contains("O&amp;T Warfighters"));
        assert!(!row.contains("O&T Warfighters"));
        assert!(row.contains("id=1234567890"));
        assert!(row.contains("data-type=\"ModContainer\""));
    }
    // --- resolve_transitive_deps (logic, no network: deps map is empty) ---
    #[test]
    fn resolve_deps_includes_missing_and_excludes_known_and_cached() {
        let missing = vec![("111".to_string(), "Mod A".to_string())];
        let known: HashSet<String> = ["222".to_string()].into_iter().collect();
        let cached: HashSet<String> = ["333".to_string()].into_iter().collect();
        // fetch_workshop_dependencies will fail (network) and return empty,
        // so only the missing root is returned. This verifies filtering
        // invariants under no-network conditions.
        let (result, deps_by_mod) = resolve_transitive_deps(&missing, &known, &cached);
        assert_eq!(result, vec![("111".to_string(), "Mod A".to_string())]);
        assert!(deps_by_mod.contains_key("111"));
    }
    // --- merge-into-mod_sources placement (byte offset logic) ---
    #[test]
    fn import_inserts_before_ignore_section() {
        let sources = "450814997 # CBA_A3\n\n[ignore]\n463939057 # ACE\n";
        let ignore_pos = sources
            .lines()
            .position(|l| {
                let l = l.trim().to_lowercase();
                l.contains("[ignore]") || l.contains("@ignore")
            })
            .map(|line_idx| {
                let mut pos = 0usize;
                for (i, line) in sources.lines().enumerate() {
                    if i == line_idx {
                        break;
                    }
                    pos += line.len() + 1;
                }
                pos
            });
        let pos = ignore_pos.unwrap();
        let mut out = sources.to_string();
        out.insert_str(pos, "1234567890 # O&T Warfighters\n");
        assert!(out.starts_with("450814997 # CBA_A3\n\n1234567890 # O&T Warfighters\n[ignore]"));
    }

    // --- TOML mod_sources (v2) ---

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
        let (mods, ignored, is_legacy) = parse_mod_sources_content(toml);
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
        let (mods, ignored, is_legacy) = parse_mod_sources_content(toml);
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
        let (mods, ignored, _) = parse_mod_sources_content(toml);
        assert!(mods.is_empty());
        assert_eq!(ignored, vec!["1234567890".to_string()]);
    }

    #[test]
    fn parse_toml_missing_name_defaults_to_mod_id() {
        let toml = r#"[[mods]]
id = "1234567890"
"#;
        let (mods, _, _) = parse_mod_sources_content(toml);
        assert_eq!(mods[0].name, "Mod 1234567890");
    }

    #[test]
    fn parse_legacy_still_works() {
        let legacy = "450814997 # CBA_A3\n887302721 # Boat Mod\n\n[ignore]\n463939057 # ACE\n";
        let (mods, ignored, is_legacy) = parse_mod_sources_content(legacy);
        assert!(is_legacy);
        assert_eq!(mods.len(), 2);
        assert_eq!(mods[0].id, "450814997");
        assert_eq!(mods[0].name, "CBA_A3");
        assert_eq!(ignored, vec!["463939057".to_string()]);
    }

    #[test]
    fn toml_migration_round_trips() {
        let legacy = "450814997 # CBA_A3\n887302721 # Boat Mod\n\n[ignore]\n463939057 # ACE\n";
        let (mods, ignored, is_legacy) = parse_mod_sources_content(legacy);
        assert!(is_legacy);

        let migrated = toml_from_legacy(&mods, &ignored);
        assert!(migrated.contains("version = 2"));
        // Ids migrate as clickable Workshop URLs
        assert!(migrated
            .contains("id = \"https://steamcommunity.com/sharedfiles/filedetails/?id=450814997\""));
        assert!(migrated.contains("name = \"CBA_A3\""));
        assert!(migrated.contains("role = \"ignore\""));
        assert!(migrated.contains("enabled = false"));

        // Migrated output parses back with identical contents
        let (mods2, ignored2, is_legacy2) = parse_mod_sources_content(&migrated);
        assert!(!is_legacy2);
        assert_eq!(mods2.len(), mods.len());
        assert_eq!(ignored2, ignored);
    }

    #[test]
    fn parse_toml_accepts_workshop_url_in_id() {
        let toml = r#"[[mods]]
id = "https://steamcommunity.com/sharedfiles/filedetails/?id=450814997"
name = "CBA_A3"
"#;
        let (mods, _, is_legacy) = parse_mod_sources_content(toml);
        assert!(!is_legacy);
        assert_eq!(mods[0].id, "450814997");
        assert_eq!(mods[0].name, "CBA_A3");
    }

    #[test]
    fn parse_toml_rejects_invalid_id() {
        // Invalid id (no 8+ digit ID, no URL) must error out. We simulate by
        // checking extract_id directly since parse_mod_sources_content exits.
        assert_eq!(extract_id("not-a-mod"), None);
        assert_eq!(extract_id("123"), None);
    }
}
