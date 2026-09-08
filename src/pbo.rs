use std::fs;
use std::path::Path;

use crate::util::extract_quoted;

// --- mod.cpp / meta.cpp parsing ---

#[derive(Debug, Clone, Default)]
pub struct ModCpp {
    pub name: String,
    pub author: String,
    pub publishedid: Option<String>,
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

pub fn get_mod_metadata(mod_dir: &Path) -> ModCpp {
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

/// Extract the addon prefix from a PBO header.
/// The prefix is the canonical virtual path (e.g. "z\ace\addons\grenades")
/// and is preserved when a pack re-packs a mod's PBO, so it identifies
/// the original addon even when the bytes differ.
pub fn pbo_prefix(path: &Path) -> Option<String> {
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

/// Derive a Workshop search term from a PBO prefix. The prefix is the
/// addon's canonical path (e.g. "TFL_Headgear" or "z\ace\addons\grenades").
/// The most distinctive token is used: for a bare prefix the first
/// underscore-separated token; for a namespaced prefix the root before
/// the first backslash. Short distinctive tokens (TFL, UKAF,
/// NAVSPECWARGRU) are what Workshop search actually matches on.
pub fn search_term_from_prefix(prefix: &str) -> String {
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
pub fn pbo_identity(path: &Path) -> Option<String> {
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
pub fn pbo_config_identity(path: &Path) -> Option<String> {
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
