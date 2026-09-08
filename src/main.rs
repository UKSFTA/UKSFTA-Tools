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
#[command(name = "uksfta", about = "UKSFTA modpack manager", version)]
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
    /// Trace the Workshop origin of PBOs not tracked in mods.lock
    Investigate {
        /// Investigate all PBOs, including tracked ones
        #[arg(long)]
        all: bool,
        /// Check each origin against the Steam Workshop API (network)
        #[arg(long)]
        online: bool,
    },
    /// Show the installed version and check for updates
    Version,
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

/// Escape a string for use inside a TOML basic string (double-quoted).
/// Mod names come from untrusted sources, so quotes, backslashes and
/// control characters must not corrupt the generated mod_sources.txt.
fn toml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
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

    let work_items: Vec<&(ModLockEntry, String, String, Vec<PathBuf>)> = planned
        .iter()
        .filter(|(_, _, status, _)| status != "unchanged")
        .collect();

    let bar = indicatif::ProgressBar::new(work_items.len() as u64);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} mods {msg}",
        )
        .unwrap()
        .progress_chars("##-"),
    );

    for (entry, id, _status, sources) in work_items {
        bar.set_message(format!("{} ({})", entry.name, id));

        fs::create_dir_all(addons_dir).expect("Failed to create addons directory");
        // files[i] is the dest path, sources[i] is the real source path
        for (dest, src) in entry.files.iter().zip(sources.iter()) {
            fs::copy(src, dest).expect("Failed to copy PBO");
        }
        new_lock.mods.insert(id.clone(), entry.clone());
        bar.inc(1);
    }
    bar.finish_and_clear();

    for (_entry, id, status, _) in &planned {
        if status == "unchanged" {
            // Keep the existing lock entry as-is
            new_lock.mods.insert(id.clone(), lock.mods[id].clone());
        }
    }

    // Print the applied changes summary
    println!(
        "\nSynced: {} added, {} updated, {} unchanged, {} removed",
        added,
        updated,
        unchanged,
        removed.len()
    );

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

    let mut mod_dirs = build_mod_dirs(&caches);

    // If the current directory is itself a Workshop mod folder, exclude it.
    if let Some(self_id) = self_workshop_id(&caches) {
        mod_dirs.retain(|d| d.id != self_id);
    }

    println!("PBO Origins:");
    let index = build_pbo_index(&mod_dirs);
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .map(|e| e == "pbo")
                .unwrap_or(false)
            {
                let name = entry.file_name().to_string_lossy().to_string();
                let target_prefix = pbo_prefix(&entry.path());
                let candidates = index
                    .get(name.as_str())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let origin =
                    resolve_pbo_from_index(&entry.path(), candidates, target_prefix.as_deref())
                        .map(|r| r.id)
                        .unwrap_or_else(|| "Unknown".to_string());
                println!("  {} -> Workshop {}", name, origin);
            }
        }
    }
}

/// SHA256 of a file's contents, hex-encoded. None on read failure.
fn file_sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).ok()?;
    Some(format!("{:x}", hasher.finalize()))
}

/// A Workshop mod folder plus precomputed statistics used to arbitrate
/// origin: how many PBOs it holds and how many distinct prefix roots
/// (a standalone mod has few roots; an aggregate pack spans many).
struct ModDir {
    id: String,
    path: PathBuf,
    pbos: Vec<PathBuf>,
    root_count: usize,
}

fn build_mod_dirs(caches: &[PathBuf]) -> Vec<ModDir> {
    let mut dirs = Vec::new();
    for cache in caches {
        if let Ok(entries) = fs::read_dir(cache) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(id) = path.file_name().and_then(|n| n.to_str()) {
                        let pbos = find_pbos(&path);
                        let root_count = {
                            let mut roots = std::collections::HashSet::new();
                            for pbo in &pbos {
                                if let Some(prefix) = pbo_prefix(pbo) {
                                    roots.insert(prefix_root(&prefix).to_lowercase());
                                }
                            }
                            roots.len()
                        };
                        dirs.push(ModDir {
                            id: id.to_string(),
                            path,
                            pbos,
                            root_count,
                        });
                    }
                }
            }
        }
    }
    dirs
}

/// The root of an addon prefix: the first path component before a
/// backslash (e.g. "z\ace\addons\grenades" -> "z"). A standalone mod's
/// PBOs share few roots; an aggregate pack spans many.
fn prefix_root(prefix: &str) -> &str {
    prefix.split('\\').next().unwrap_or(prefix)
}

/// The Workshop ID of the current directory, if the current directory is
/// itself a mod folder inside a Workshop cache (e.g. investigating a pack's
/// own contents). Such a folder must not be a candidate for its own PBOs.
fn self_workshop_id(caches: &[PathBuf]) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let cwd = fs::canonicalize(&cwd).ok()?;
    for cache in caches {
        let cache = fs::canonicalize(cache).ok()?;
        if cwd.starts_with(&cache) {
            // The mod folder is cwd relative to cache
            if let Ok(rel) = cwd.strip_prefix(&cache) {
                let components: Vec<_> = rel.components().collect();
                if components.len() == 1 {
                    return components[0].as_os_str().to_str().map(|s| s.to_string());
                }
            }
        }
    }
    None
}

/// Extract the addon prefix from a PBO header.
/// The prefix is the canonical virtual path (e.g. "z\ace\addons\grenades")
/// and is preserved when a pack re-packs a mod's PBO, so it identifies
/// the original addon even when the bytes differ.
fn pbo_prefix(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut file = fs::File::open(path).ok()?;
    let mut data = [0u8; 512];
    let n = file.read(&mut data).ok()?;
    let data = &data[..n];

    // Header entries are null-separated key\0value\0 pairs. Find "prefix".
    let key = b"prefix\x00";
    let idx = data.windows(key.len()).position(|w| w == key)?;
    let start = idx + key.len();
    let end = data[start..].iter().position(|&b| b == 0)? + start;
    let prefix = data[start..end].to_vec();
    String::from_utf8(prefix).ok()
}

/// The resolved origin of a PBO: the Workshop ID plus the prefix that
/// identified it, and whether the match was itself an aggregate pack.
#[derive(Debug, PartialEq, Clone)]
struct ResolvedOrigin {
    id: String,
    prefix: Option<String>,
    is_pack: bool,
}

/// The local identity cache type: search term -> confirmed (id, title)
/// pairs, persisted to .uksfta/identities.json.
type IdentityCache = HashMap<String, Vec<(String, String)>>;

/// Build a single-pass index of every PBO name to the mod folders that
/// contain it. This is the expensive cache walk; resolving then becomes
/// in-memory lookups instead of re-scanning folders per PBO.
fn build_pbo_index(mod_dirs: &[ModDir]) -> HashMap<String, Vec<&ModDir>> {
    let mut index: HashMap<String, Vec<&ModDir>> = HashMap::new();
    for dir in mod_dirs {
        for pbo in &dir.pbos {
            if let Some(name) = pbo.file_name().and_then(|n| n.to_str()) {
                index.entry(name.to_string()).or_default().push(dir);
            }
        }
    }
    index
}

/// Match a PBO against the Workshop cache and arbitrate its origin.
/// Primary signal: the PBO header prefix, which identifies the original
/// addon and survives re-packing. Byte-hash is a fallback for PBOs
/// without a readable prefix.
///
/// Among folders carrying the same prefix, prefer the one with the fewest
/// distinct prefix roots: a standalone mod (e.g. ACE) has one root, while
/// an aggregate pack spans many. This steers the result to the original
/// owner mod over any pack that bundled a copy.
///
/// `candidates` is the prebuilt per-name folder list from the index.
fn resolve_pbo_from_index(
    pbo_path: &Path,
    candidates: &[&ModDir],
    target_prefix: Option<&str>,
) -> Option<ResolvedOrigin> {
    let pbo_name = pbo_path.file_name()?.to_string_lossy().to_string();

    // Score each candidate. Prefix match is the primary signal and is
    // cheap (512-byte header read). Byte-hash is a fallback used only
    // when the prefix path fails, since hashing reads the whole PBO.
    let mut scored: Vec<(&ModDir, u8)> = Vec::new();
    for dir in candidates {
        let cached_path = dir.pbos.iter().find(|pbo| {
            pbo.file_name()
                .map(|n| n == pbo_name.as_str())
                .unwrap_or(false)
        })?;
        let mut score = 0u8;
        if let Some(target) = target_prefix {
            if pbo_prefix(cached_path).as_deref() == Some(target) {
                score += 2; // same canonical addon prefix
            }
        } else {
            // No prefix available: fall back to byte identity.
            let target_hash = file_sha256(pbo_path)?;
            if file_sha256(cached_path) == Some(target_hash) {
                score += 1;
            }
        }
        if score > 0 {
            scored.push((dir, score));
        }
    }

    if scored.is_empty() {
        return None;
    }
    let best_score = scored.iter().map(|(_, s)| *s).max().unwrap_or(0);

    // Among the highest-scoring folders, pick the one with the fewest
    // distinct prefix roots (standalone mod over aggregate pack).
    let best = scored
        .into_iter()
        .filter(|(_, s)| *s == best_score)
        .min_by_key(|(dir, _)| dir.root_count)?
        .0;

    // A folder is an aggregate pack when it spans a very large number of
    // distinct prefix roots. Large standalone mods (e.g. FZA AH-64 with 28
    // roots) stay below the threshold; the known aggregate packs here span
    // 61 and 159 roots. This flag is a warning to verify, not a guarantee.
    let is_pack = best.root_count > 50;

    Some(ResolvedOrigin {
        id: best.id.clone(),
        prefix: target_prefix.map(|s| s.to_string()),
        is_pack,
    })
}

/// Trace the Workshop origin of untracked PBOs in addons/.
fn investigate(all: bool, online: bool) {
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

    let mut mod_dirs = build_mod_dirs(&caches);

    // If the current directory is itself a Workshop mod folder (e.g. we are
    // investigating a pack's own contents), exclude it from candidates.
    // A folder cannot be the origin of its own PBOs.
    if let Some(self_id) = self_workshop_id(&caches) {
        println!(
            "Investigating Workshop mod {} — excluding it as a candidate.",
            self_id
        );
        mod_dirs.retain(|d| d.id != self_id);
    }

    // Tracked PBO filenames: from mods.lock if present, else all are untracked
    let lock_path = Path::new("mods.lock");
    let lock = load_lock(lock_path);
    let tracked: HashSet<String> = lock
        .mods
        .values()
        .flat_map(|m| m.files.iter())
        .filter_map(|f| {
            Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .collect();

    let mut results: Vec<(String, Option<ResolvedOrigin>)> = Vec::new(); // (pbo, origin)

    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "pbo").unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                if !all && tracked.contains(&name) {
                    continue;
                }
                results.push((name, None));
            }
        }
    }

    if results.is_empty() {
        println!("No untracked PBOs in addons/.");
        return;
    }

    // Build a single-pass index of every cached PBO name to its folders
    let index = build_pbo_index(&mod_dirs);

    // Match each untracked PBO against the index and arbitrate the origin
    for (pbo_name, origin) in &mut results {
        let pbo_path = addons_dir.join(pbo_name.as_str());
        let target_prefix = pbo_prefix(&pbo_path);
        let candidates = index
            .get(pbo_name.as_str())
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        *origin = resolve_pbo_from_index(&pbo_path, candidates, target_prefix.as_deref());
    }

    // Report
    println!("Untracked PBO Investigation:");
    println!("  {} PBO(s) examined", results.len());
    let mut unknown_count = 0;
    for (pbo, origin) in &results {
        let Some(origin) = origin else {
            unknown_count += 1;
            println!("  [UNKNOWN] {}", pbo);
            continue;
        };
        let id = &origin.id;
        let meta = mod_dirs
            .iter()
            .find(|d| &d.id == id)
            .map(|d| get_mod_metadata(&d.path))
            .unwrap_or_default();
        let name = if meta.name.is_empty() {
            format!("Mod {}", id)
        } else {
            meta.name
        };
        if origin.is_pack {
            println!(
                "  {} -> {} ({}) {} [pack: prefix {} — verify source]",
                pbo,
                name,
                id,
                workshop_url(id),
                origin.prefix.as_deref().unwrap_or("unknown")
            );
        } else {
            println!("  {} -> {} ({}) {}", pbo, name, id, workshop_url(id));
        }
    }
    if unknown_count > 0 {
        println!(
            "\n{} PBO(s) could not be matched to any Workshop cache entry.",
            unknown_count
        );
        println!("They may come from a deleted or private mod, or be manually placed.");
    }

    if online {
        check_workshop_visibility(results.clone());

        // For PBOs with no local origin OR whose origin is itself an aggregate
        // pack, search the Workshop by prefix and report the best candidate
        // matches. This is a best-effort search: Workshop text search is
        // imprecise, so candidates are shown for the user to verify rather
        // than asserted.
        let searchable: Vec<(String, String)> = results
            .iter()
            .filter(|(_, origin)| match origin {
                None => true,
                Some(o) => o.is_pack,
            })
            .map(|(name, _)| {
                let pbo_path = addons_dir.join(name);
                // Prefer the richest identity in order:
                // 1. requiredAddons root from plain-text config (certified
                //    mod family: rhsusf, MRHMilsimTools)
                // 2. string-table token from config ($STR_RHSUSF_... ->
                //    RHSUSF)
                // 3. short author handle from config (DANZ, TFB)
                // 4. header prefix (last resort)
                let term = pbo_config_identity(&pbo_path)
                    .or_else(|| pbo_identity(&pbo_path))
                    .or_else(|| pbo_prefix(&pbo_path))
                    .unwrap_or_else(|| name.clone());
                (name.clone(), term)
            })
            .collect();

        if !searchable.is_empty() {
            println!(
                "\nSearching Workshop for {} PBO(s) whose origin is unknown or a pack...",
                searchable.len()
            );
            let mut searched = HashSet::new();
            let mut cache = load_identity_cache();
            let api_client = reqwest::blocking::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("Failed to build HTTP client");
            for (name, term) in searchable {
                let term = search_term_from_prefix(&term);
                if searched.contains(&term) {
                    continue;
                }
                searched.insert(term.clone());

                // Check the local cache first; only hit the Workshop for
                // terms we have not already searched.
                let (titles, from_cache): (Vec<(String, String)>, bool) =
                    if let Some(cached) = cache.get(&term) {
                        println!("  {} (search \"{}\"): from cache", name, term);
                        (cached.clone(), true)
                    } else {
                        let candidates = search_workshop(&term);
                        let confirmed = batch_workshop_titles(&candidates, &api_client);
                        cache.insert(term.clone(), confirmed.clone());
                        save_identity_cache(&cache);
                        (confirmed, false)
                    };

                if titles.is_empty() {
                    println!("  {}: no candidates for \"{}\"", name, term);
                } else {
                    let shown = titles
                        .iter()
                        .take(3)
                        .map(|(id, title)| format!("{} ({})", title, id))
                        .collect::<Vec<_>>();
                    println!(
                        "  {} (search \"{}\"): {} candidate(s) — {}",
                        name,
                        term,
                        titles.len(),
                        shown.join(", ")
                    );
                }
                // Rate-limit only real Workshop page hits, not cache reads.
                if !from_cache {
                    std::thread::sleep(std::time::Duration::from_millis(1100));
                }
            }
        }
    }

    /// The local identity cache: maps a PBO search term to the confirmed
    /// (id, title) pairs found for it. Persisted to .uksfta/identities.json
    /// so repeat investigations reuse prior searches entirely offline. This
    /// file is gitignored and never leaves the machine.
    fn identity_cache_path() -> PathBuf {
        Path::new(".uksfta").join("identities.json")
    }

    fn load_identity_cache() -> IdentityCache {
        let path = identity_cache_path();
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return HashMap::new(),
        };
        serde_json::from_str(&content).unwrap_or_default()
    }

    fn save_identity_cache(cache: &IdentityCache) {
        let path = identity_cache_path();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(cache) {
            let _ = fs::write(&path, json);
        }
    }

    /// Batch-confirm a list of Workshop item IDs via the keyless
    /// GetPublishedFileDetails API. Returns (id, title) pairs for the items
    /// that exist. Empty on network or parse failure.
    fn batch_workshop_titles(
        ids: &[String],
        client: &reqwest::blocking::Client,
    ) -> Vec<(String, String)> {
        let mut form = String::from("itemcount=");
        form.push_str(&ids.len().to_string());
        for (i, id) in ids.iter().enumerate() {
            form.push_str(&format!("&publishedfileids[{}]={}", i, id));
        }
        let body = match client
            .post("https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .ok()
            .and_then(|r| r.text().ok())
        {
            Some(b) => b,
            None => return Vec::new(),
        };

        #[derive(serde::Deserialize)]
        struct ApiResponse {
            response: ResponseInner,
        }
        #[derive(serde::Deserialize)]
        struct ResponseInner {
            publishedfiledetails: Vec<FileDetail>,
        }
        #[derive(serde::Deserialize)]
        struct FileDetail {
            publishedfileid: String,
            result: u32,
            #[serde(default)]
            title: String,
        }

        let parsed: ApiResponse = match serde_json::from_str(&body) {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
        parsed
            .response
            .publishedfiledetails
            .into_iter()
            .filter(|d| d.result == 1)
            .map(|d| (d.publishedfileid, d.title))
            .collect()
    }
}

/// Derive a Workshop search term from a PBO prefix. The prefix is the
/// addon's canonical path (e.g. "TFL_Headgear" or "z\ace\addons\grenades").
/// The most distinctive token is used: for a bare prefix the first
/// underscore-separated token; for a namespaced prefix the root before
/// the first backslash. Short distinctive tokens (TFL, UKAF,
/// NAVSPECWARGRU) are what Workshop search actually matches on.
fn search_term_from_prefix(prefix: &str) -> String {
    // Split the namespace: for "z\ace\addons\grenades" the parts are
    // z, ace, addons, grenades. The mod identity is usually the second
    // component ("ace"); the first ("z") is a generic convention.
    let parts: Vec<&str> = prefix.split('\\').collect();
    let candidate = if parts.len() >= 2 {
        // "z\ace" -> "ace"; skip a too-short first component
        if parts[0].len() < 3 && parts[1].len() >= 3 {
            parts[1]
        } else {
            parts[0]
        }
    } else {
        parts[0]
    };
    let first = candidate.split('_').next().unwrap_or(candidate);
    // Use the first underscore token when it is a plausible mod identity;
    // otherwise fall back to the full candidate. No upper cap: mod names
    // like NAVSPECWARGRU2 legitimately exceed a short token limit.
    if first.len() >= 3 {
        first.to_string()
    } else {
        candidate.to_string()
    }
}

/// Extract a mod identity from a PBO's packed config content.
/// The config carries richer identity than the header prefix: mod-family
/// string-table tokens ("$STR_RHSUSF_AUTHOR_FULL" -> "RHSUSF") and short
/// author handles ("DANZ", "KM", "TFB").
///
/// The Workshop search matches mod TITLES, not authors, so the most
/// effective term is a short distinctive token that appears in the mod's
/// title. A long author name ("UnderSiege Productionz") is ignored: the
/// mod is titled "USP Gear", not the author's name.
/// Returns the best identity token found, or None.
fn pbo_identity(path: &Path) -> Option<String> {
    use std::io::Read;
    // Read up to the first 2 MB of the PBO: the header and config are at
    // the start, and config content rarely exceeds this.
    let file = fs::File::open(path).ok()?;
    let mut data = Vec::new();
    file.take(2_000_000).read_to_end(&mut data).ok()?;

    // Readable strings >= 6 chars
    let strings: Vec<String> = data
        .split(|&b| !(0x20..=0x7e).contains(&b))
        .filter(|s| s.len() >= 6)
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect();

    // 1. Prefer a mod-family string-table token: $STR_<MODID>_...
    //    This is the mod's internal identifier (RHSUSF, MRH, ACE) and
    //    matches mod titles far better than authors.
    for s in &strings {
        if let Some(idx) = s.find("$STR_") {
            let token = &s[idx + 5..];
            let token: String = token
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            // The mod family is the first underscore segment: from
            // "RHSUSF_AUTHOR_FULL" take "RHSUSF".
            let token = token.split('_').next().unwrap_or(&token);
            if token.len() >= 3 && !["BIS", "AR", "CORE"].contains(&token) {
                return Some(token.to_string());
            }
        }
    }

    // 2. Fall back to a short author handle: author = "Name" where the
    //    name is a single short token likely to appear in the mod title
    //    (DANZ, KM, TFB, MARKO). Skip long names and $STR_ placeholders.
    for s in &strings {
        if let Some(idx) = s.find("author") {
            let after = &s[idx + 6..];
            let after = after.trim_start();
            if let Some(after) = after.strip_prefix('=') {
                let after = after.trim_start();
                if let Some(after) = after.strip_prefix('"') {
                    if let Some(end) = after.find('"') {
                        let name = after[..end].trim();
                        // Author handles must be distinctive enough to
                        // appear in the mod's Workshop title. Require 3+
                        // chars so generic 2-letter handles (KM) do not
                        // mask a better prefix search.
                        if !name.starts_with('$')
                            && name.len() >= 3
                            && name.len() <= 12
                            && !name.contains([' ', '&', '.', '\''])
                            && !["AUTHOR", "SITREP", "UNKNOWN"].contains(&name)
                        {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Extract identity from a PBO's plain-text config.cpp content.
/// Most pack PBOs store config.cpp unpacked (131 of 198 verified), so the
/// full CfgPatches block is readable: `requiredAddons[]` names the exact
/// addons the mod depends on (e.g. "rhsusf_c_weapons" -> the RHS USF mod
/// family) and `author`/`name` give the mod's own identity.
///
/// Returns the most distinctive identity: a non-vanilla required addon
/// root (MRHMilsimTools, rhsusf, ace), then the CfgPatches author if it
/// is a short single-word handle. None if the config is not readable.
fn pbo_config_identity(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = fs::File::open(path).ok()?;
    let mut data = Vec::new();
    file.take(4_000_000).read_to_end(&mut data).ok()?;

    // Readable strings, preserving config.cpp structure
    let strings: Vec<String> = data
        .split(|&b| !(0x20..=0x7e).contains(&b))
        .filter(|s| s.len() >= 4)
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect();
    let text = strings.join("\n");

    // 1. requiredAddons[] = { "A3_Weapons_F", "rhsusf_c_weapons", ... }
    //    The non-vanilla entries name the mod families this addon needs.
    //    Match the exact field (not a loose "requiredAddons" + brace that
    //    could hit an unrelated class body).
    if let Some(start) = text.find("requiredAddons[]") {
        let after = &text[start + "requiredAddons[]".len()..];
        let after = after.trim_start();
        if let Some(after) = after.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(after) = after.strip_prefix('{') {
                let after = after.trim_start();
                if let Some(close) = after.find('}') {
                    let body = &after[..close];
                    for addon in body.split(',') {
                        // Normalise: the string extraction may split an addon
                        // across a newline ("A\n3_Weapons"), so strip all
                        // whitespace before comparing.
                        let addon: String = addon
                            .chars()
                            .filter(|c| !c.is_whitespace() && *c != '"')
                            .collect();
                        // Take the root before the first underscore: the mod
                        // family (rhsusf_c_weapons -> rhsusf).
                        let root = addon.split('_').next().unwrap_or(&addon);
                        // Skip vanilla (A3_*) and common base addons
                        // (cba_main -> cba) — they are not the mod's
                        // identity.
                        if root.starts_with("A3")
                            || root == "cba"
                            || root == "CuratorOnly"
                            || root == "ace"
                            || root.len() < 3
                        {
                            continue;
                        }
                        return Some(root.to_string());
                    }
                }
            }
        }
    }

    None
}

/// Search the Steam Workshop browse page for a query and return candidate
/// item IDs. Returns an empty vec on network or parse failure.
fn search_workshop(query: &str) -> Vec<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("Failed to build HTTP client");
    let url = format!(
        "https://steamcommunity.com/workshop/browse/?appid=107410&searchtext={}",
        urlencode(query)
    );
    let html = match client.get(&url).send().ok().and_then(|r| r.text().ok()) {
        Some(h) => h,
        None => return Vec::new(),
    };
    // The browse page links items as filedetails/?id=NNN. Deduplicate.
    let mut ids = Vec::new();
    for id in html.split("filedetails/?id=").skip(1) {
        let id: String = id.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
            if ids.len() >= 10 {
                break;
            }
        }
    }
    ids
}

/// Percent-encode a query string for the Workshop browse URL.
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Query the Steam Workshop API for each investigated mod's visibility.
/// Keyless: ISteamRemoteStorage/GetPublishedFileDetails needs no API key.
fn check_workshop_visibility(results: Vec<(String, Option<ResolvedOrigin>)>) {
    // Collect unique origin IDs
    let mut ids: Vec<String> = results
        .iter()
        .filter_map(|(_, origin)| origin.as_ref().map(|o| o.id.clone()))
        .collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        println!("\nOnline check: no origins to check.");
        return;
    }

    println!("\nChecking {} mod(s) against Steam Workshop...", ids.len());
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("Failed to build HTTP client");

    let mut form = String::from("itemcount=");
    form.push_str(&ids.len().to_string());
    for (i, id) in ids.iter().enumerate() {
        form.push_str(&format!("&publishedfileids[{}]={}", i, id));
    }

    let body = match client
        .post("https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(form)
        .send()
    {
        Ok(r) => match r.text() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Error reading API response: {}", e);
                return;
            }
        },
        Err(e) => {
            eprintln!("Error calling Steam API: {}", e);
            return;
        }
    };

    // Parse the response. Result 1 = exists (public or visible), 9 = not
    // publicly visible (removed or private).
    #[derive(serde::Deserialize)]
    struct ApiResponse {
        response: ResponseInner,
    }
    #[derive(serde::Deserialize)]
    struct ResponseInner {
        publishedfiledetails: Vec<FileDetail>,
    }
    #[derive(serde::Deserialize)]
    struct FileDetail {
        publishedfileid: String,
        result: u32,
        #[serde(default)]
        title: String,
        #[serde(default)]
        creator: String,
    }

    let parsed: ApiResponse = match serde_json::from_str(&body) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error parsing API response: {}", e);
            return;
        }
    };

    for detail in parsed.response.publishedfiledetails {
        let status = if detail.result == 1 {
            format!("PUBLIC (by {})", detail.creator)
        } else {
            "NOT PUBLICLY VISIBLE (removed or private)".to_string()
        };
        let title = if detail.title.is_empty() {
            detail.publishedfileid.clone()
        } else {
            detail.title
        };
        println!("  {} -> {} [{}]", detail.publishedfileid, title, status);
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

/// Show the installed version and check GitHub for a newer release.
/// Prints update instructions matching the install method in use.
fn version() {
    let current = env!("CARGO_PKG_VERSION");
    println!("uksfta {}", current);

    // Query the latest release tag from GitHub (no key required)
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build HTTP client");

    let body = match client
        .get("https://api.github.com/repos/UKSFTA/UKSFTA-Tools/releases/latest")
        .header("User-Agent", "uksfta")
        .send()
    {
        Ok(r) => match r.text() {
            Ok(t) => t,
            Err(e) => {
                println!("Could not check for updates: {}", e);
                return;
            }
        },
        Err(e) => {
            println!("Could not check for updates: {}", e);
            println!("Offline or no access to GitHub.");
            return;
        }
    };

    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            println!("Could not parse update response: {}", e);
            return;
        }
    };

    // Tags are "vX.Y.Z"; strip the leading 'v' for comparison
    let latest = release.tag_name.trim_start_matches('v');
    if !is_outdated(current, latest) {
        println!("You are up to date.");
        return;
    }

    println!(
        "\nA new version is available: v{} (you have v{})",
        latest, current
    );
    println!("Update with:");
    if cfg!(target_os = "windows") {
        println!(
            "  irm https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.ps1 | iex"
        );
    } else {
        println!(
            "  curl -fsSL https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.sh | sh"
        );
    }
}

/// Compare two dotted version strings. Returns true when `installed` is
/// older than `latest`. Handles the "v" prefix. Non-numeric parts are
/// ignored for the comparison.
fn is_outdated(installed: &str, latest: &str) -> bool {
    let installed: Vec<u32> = installed
        .trim_start_matches('v')
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();
    let latest: Vec<u32> = latest
        .trim_start_matches('v')
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();

    for (a, b) in installed.iter().zip(latest.iter()) {
        if a != b {
            return a < b;
        }
    }
    installed.len() < latest.len()
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
        Commands::Investigate { all, online } => investigate(all, online),
        Commands::Version => version(),
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

    #[test]
    fn modlist_row_html_escapes_html_metacharacters() {
        // A malicious Workshop name must not inject markup into the HTML.
        let row = modlist_row_html("1234567890", "<script>alert(1)</script>");
        assert!(!row.contains("<script>"));
        assert!(row.contains("&lt;script&gt;"));
        let quoted = modlist_row_html("1234567890", "Mod \"quoted\"");
        assert!(quoted.contains("Mod &quot;quoted&quot;"));
        assert!(!quoted.contains("Mod \"quoted\""));
    }

    #[test]
    fn toml_escape_handles_quotes_backslashes_and_controls() {
        assert_eq!(toml_escape("plain"), "plain");
        assert_eq!(toml_escape("a\"b"), "a\\\"b");
        assert_eq!(toml_escape("a\\b"), "a\\\\b");
        assert_eq!(toml_escape("a\nb"), "a\\nb");
        assert_eq!(toml_escape("a\tb"), "a\\tb");
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

    // --- investigate ---

    #[test]
    fn file_sha256_matches_known_value() {
        let dir = std::env::temp_dir().join("uksfta-sha-test");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("data.bin");
        fs::write(&f, b"hello world").unwrap();
        // sha256 of "hello world"
        assert_eq!(
            file_sha256(&f).unwrap(),
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn file_sha256_missing_file_returns_none() {
        assert_eq!(
            file_sha256(&Path::new("/nonexistent/uksfta/test.pbo")),
            None
        );
    }

    #[test]
    fn resolve_pbo_origin_prefers_standalone_over_pack() {
        let dir = std::env::temp_dir().join("uksfta-origin-test");
        fs::create_dir_all(&dir).unwrap();

        // mod 111 (standalone, 1 PBO) and mod 222 (pack, 50 PBOs) both
        // contain an identical copy of common.pbo
        let standalone = dir.join("111").join("addons");
        let pack = dir.join("222").join("addons");
        fs::create_dir_all(&standalone).unwrap();
        fs::create_dir_all(&pack).unwrap();
        let content = b"identical pbo bytes";
        fs::write(standalone.join("common.pbo"), content).unwrap();
        fs::write(pack.join("common.pbo"), content).unwrap();
        for i in 0..50 {
            fs::write(pack.join(format!("pack_{}.pbo", i)), format!("pack{}", i)).unwrap();
        }

        let mod_dirs = vec![
            ModDir {
                id: "222".to_string(),
                path: dir.join("222"),
                pbos: find_pbos(&dir.join("222")),
                root_count: 50,
            },
            ModDir {
                id: "111".to_string(),
                path: dir.join("111"),
                pbos: find_pbos(&dir.join("111")),
                root_count: 1,
            },
        ];
        let index = build_pbo_index(&mod_dirs);

        let target = dir.join("common.pbo");
        fs::write(&target, content).unwrap();

        // Arbitration must pick the standalone mod (111) with fewer roots
        let candidates = index.get("common.pbo").map(|v| v.as_slice()).unwrap_or(&[]);
        assert_eq!(
            resolve_pbo_from_index(&target, candidates, pbo_prefix(&target).as_deref())
                .unwrap()
                .id,
            "111".to_string()
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_pbo_origin_no_match_returns_none() {
        let dir = std::env::temp_dir().join("uksfta-origin-none");
        fs::create_dir_all(&dir).unwrap();
        let mod_dir = dir.join("111").join("addons");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("other.pbo"), b"different").unwrap();

        let target = dir.join("target.pbo");
        fs::write(&target, b"no match anywhere").unwrap();

        let mod_dirs = vec![ModDir {
            id: "111".to_string(),
            path: dir.join("111"),
            pbos: find_pbos(&dir.join("111")),
            root_count: 1,
        }];
        let index = build_pbo_index(&mod_dirs);
        let candidates = index.get("target.pbo").map(|v| v.as_slice()).unwrap_or(&[]);
        assert_eq!(
            resolve_pbo_from_index(&target, candidates, pbo_prefix(&target).as_deref()),
            None
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    // --- PBO prefix and prefix-based origin resolution ---

    /// Write a fake PBO with a header carrying the given prefix.
    fn write_pbo(path: &Path, prefix: &str, payload: &[u8]) {
        // Mimic the real PBO header: b"\x00sreV" then "prefix\0{prefix}\0".
        let mut data = b"\x00sreV\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        data.extend_from_slice(b"prefix\x00");
        data.extend_from_slice(prefix.as_bytes());
        data.push(0);
        data.extend_from_slice(payload);
        fs::write(path, data).unwrap();
    }

    #[test]
    fn pbo_prefix_extracts_canonical_path() {
        let dir = std::env::temp_dir().join("uksfta-prefix-test");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo(&f, "z\\ace\\addons\\grenades", b"data");
        assert_eq!(
            pbo_prefix(&f).unwrap(),
            "z\\ace\\addons\\grenades".to_string()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_prefix_no_prefix_returns_none() {
        let dir = std::env::temp_dir().join("uksfta-prefix-none");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        fs::write(&f, b"\x00sreVno prefix here").unwrap();
        assert_eq!(pbo_prefix(&f), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_pbo_origin_matches_by_prefix_over_differing_bytes() {
        // Two folders hold same-named PBOs with the same prefix but
        // different bytes (a pack repacked the mod's copy). The origin
        // must be the folder with fewer roots (the standalone mod).
        let dir = std::env::temp_dir().join("uksfta-prefix-origin");
        fs::create_dir_all(&dir).unwrap();
        let standalone = dir.join("111").join("addons");
        let pack = dir.join("222").join("addons");
        fs::create_dir_all(&standalone).unwrap();
        fs::create_dir_all(&pack).unwrap();
        write_pbo(
            &standalone.join("common.pbo"),
            "z\\ace\\addons\\common",
            b"version-a",
        );
        write_pbo(
            &pack.join("common.pbo"),
            "z\\ace\\addons\\common",
            b"version-b",
        );

        let mod_dirs = vec![
            ModDir {
                id: "222".to_string(),
                path: dir.join("222"),
                pbos: find_pbos(&dir.join("222")),
                root_count: 50,
            },
            ModDir {
                id: "111".to_string(),
                path: dir.join("111"),
                pbos: find_pbos(&dir.join("111")),
                root_count: 1,
            },
        ];
        let index = build_pbo_index(&mod_dirs);

        let target = dir.join("common.pbo");
        write_pbo(&target, "z\\ace\\addons\\common", b"version-a");

        let candidates = index.get("common.pbo").map(|v| v.as_slice()).unwrap_or(&[]);
        let origin =
            resolve_pbo_from_index(&target, candidates, pbo_prefix(&target).as_deref()).unwrap();
        assert_eq!(origin.id, "111".to_string());
        assert!(!origin.is_pack);

        fs::remove_dir_all(&dir).unwrap();
    }

    // --- version comparison ---

    #[test]
    fn search_term_from_prefix_uses_distinctive_token() {
        // Bare prefix: first underscore token is the mod identity
        assert_eq!(search_term_from_prefix("TFL_Headgear"), "TFL");
        assert_eq!(
            search_term_from_prefix("NAVSPECWARGRU2_TACDEV"),
            "NAVSPECWARGRU2"
        );
        // Namespaced prefix: second component is the mod identity
        assert_eq!(search_term_from_prefix("z\\ace\\addons\\grenades"), "ace");
        // Too-generic root falls back to the full root
        assert_eq!(search_term_from_prefix("z"), "z");
        assert_eq!(search_term_from_prefix("x\\zen\\addons\\ai"), "zen");
    }

    #[test]
    fn urlencode_encodes_spaces_and_symbols() {
        assert_eq!(urlencode("TFL Headgear"), "TFL%20Headgear");
        assert_eq!(urlencode("a/b&c"), "a%2Fb%26c");
        assert_eq!(urlencode("ACE"), "ACE");
    }

    /// Write a fake PBO whose packed config contains the given text.
    fn write_pbo_with_config(path: &Path, config: &str) {
        let mut data = b"\x00sreV\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        data.extend_from_slice(b"prefix\x00test\x00");
        data.extend_from_slice(config.as_bytes());
        fs::write(path, data).unwrap();
    }

    #[test]
    fn pbo_identity_prefers_string_table_token() {
        let dir = std::env::temp_dir().join("uksfta-identity-token");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        // A $STR_ token (mod family) beats a long author name
        write_pbo_with_config(
            &f,
            r#"author = "Red Hammer Studios"; author = "$STR_RHSUSF_AUTHOR_FULL";"#,
        );
        assert_eq!(pbo_identity(&f).unwrap(), "RHSUSF");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_identity_falls_back_to_short_author() {
        let dir = std::env::temp_dir().join("uksfta-identity-author");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo_with_config(&f, r#"author = "DANZ";"#);
        assert_eq!(pbo_identity(&f).unwrap(), "DANZ");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_identity_rejects_long_author_names() {
        let dir = std::env::temp_dir().join("uksfta-identity-long");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        // Long multi-word author names don't match mod titles -> ignored
        write_pbo_with_config(&f, r#"author = "UnderSiege Productionz";"#);
        assert_eq!(pbo_identity(&f), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_identity_rejects_two_char_author_handles() {
        // Two-letter handles (KM) are too generic to search by; the
        // prefix should win instead.
        let dir = std::env::temp_dir().join("uksfta-identity-km");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo_with_config(&f, r#"author = "KM";"#);
        assert_eq!(pbo_identity(&f), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_config_identity_extracts_required_addon_root() {
        let dir = std::env::temp_dir().join("uksfta-config-identity");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        // A config with requiredAddons naming a mod family
        write_pbo_with_config(
            &f,
            r#"class CfgPatches { requiredAddons[] = { "A3_Weapons_F", "rhsusf_c_weapons" }; };"#,
        );
        assert_eq!(pbo_config_identity(&f).unwrap(), "rhsusf");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_config_identity_skips_vanilla_and_cba() {
        let dir = std::env::temp_dir().join("uksfta-config-vanilla");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        // Only vanilla + cba deps: not a usable identity
        write_pbo_with_config(
            &f,
            r#"class CfgPatches { requiredAddons[] = { "A3_Weapons_F", "cba_main" }; };"#,
        );
        assert_eq!(pbo_config_identity(&f), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn identity_cache_round_trips() {
        let dir = std::env::temp_dir().join("uksfta-idcache-test");
        fs::create_dir_all(&dir).unwrap();
        // Point the cache at the test dir so we do not touch .uksfta in
        // the workspace; run the round-trip via direct file IO.
        let path = dir.join("identities.json");
        let mut cache: IdentityCache = HashMap::new();
        cache.insert(
            "TFL".to_string(),
            vec![(
                "3797815099".to_string(),
                "@THE TFL AIO CAG PACK".to_string(),
            )],
        );
        let json = serde_json::to_string_pretty(&cache).unwrap();
        fs::write(&path, json).unwrap();
        let loaded: IdentityCache =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            loaded.get("TFL").unwrap(),
            &vec![(
                "3797815099".to_string(),
                "@THE TFL AIO CAG PACK".to_string()
            )]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn is_outdated_detects_newer_major() {
        assert!(is_outdated("0.1.0", "0.2.0"));
        assert!(is_outdated("1.0.0", "2.0.0"));
    }

    #[test]
    fn is_outdated_detects_newer_patch() {
        assert!(is_outdated("0.2.0", "0.2.1"));
    }

    #[test]
    fn is_outdated_same_version_is_false() {
        assert!(!is_outdated("0.2.0", "0.2.0"));
        assert!(!is_outdated("v0.2.0", "0.2.0"));
    }

    #[test]
    fn is_outdated_newer_installed_is_false() {
        assert!(!is_outdated("0.3.0", "0.2.0"));
    }
}
