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

/// The trimmed left side of an `=` assignment, or None when the line has
/// no `=`. Exact-key matching stops `namespace` from matching `name`.
fn key_of(line: &str) -> Option<&str> {
    line.split_once('=').map(|(key, _)| key.trim())
}

fn parse_mod_cpp(path: &Path) -> ModCpp {
    let mut result = ModCpp::default();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = extract_quoted(line) {
                match key_of(line) {
                    Some("name") => result.name = val,
                    Some("author") => result.author = val,
                    _ => {}
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
                match key_of(line) {
                    Some("publishedid") => result.publishedid = Some(val),
                    Some("name") => result.name = val,
                    _ => {}
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
    let parts: Vec<&str> = crate::prefix::split_backslash(prefix);
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
    let first = crate::prefix::first_underscore_token(candidate);
    // Use the first underscore token when it is a plausible mod identity;
    // otherwise fall back to the full candidate. No upper cap: mod names
    // like NAVSPECWARGRU2 legitimately exceed a short token limit.
    if first.len() >= 3 {
        first.to_string()
    } else {
        candidate.to_string()
    }
}

/// Extract additional search terms from the full prefix path.
/// For "x\SPS\Vehicles\sps_blackhornet", the primary term is "SPS" but
/// we also want to search "sps_blackhornet" and "blackhornet" — the
/// distinctive parts of the prefix that Workshop search might match.
pub fn extra_search_terms_from_prefix(prefix: &str) -> Vec<String> {
    let parts: Vec<&str> = prefix.split('\\').collect();
    let mut extras = Vec::new();
    let skip = ["addons", "scripts", "functions", "models", "data", "config"];
    for part in &parts {
        if part.len() >= 4 && !skip.contains(part) {
            extras.push(part.to_string());
        }
    }
    // Also try the last two segments joined (e.g. "Vehicles_sps_blackhornet")
    if parts.len() >= 3 {
        let tail = parts[parts.len() - 2..].join("_");
        if tail.len() >= 4 {
            extras.push(tail);
        }
    }
    extras
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
/// Structured data extracted from a PBO's CfgPatches config block.
/// Used for cross-referencing PBOs against Workshop candidates.
#[derive(Debug, Clone, Default)]
pub struct CfgPatchesInfo {
    /// The CfgPatches class name (e.g. "ffaa_data", "ade").
    /// This is the addon's identity — used as a search term.
    pub name: Option<String>,
    /// The `author` field: mod author name (e.g. "UnderSiege Productionz").
    pub author: Option<String>,
    /// The `url` field: sometimes a direct Workshop page URL.
    pub url: Option<String>,
    /// Required addons from `requiredAddons[]`: dependency mod families.
    pub required_addons: Vec<String>,
}

/// Extract structured CfgPatches data from a PBO's packed config.
/// Returns author, URL, and required addons list. Free cross-reference
/// signals: author matches creator_name from Workshop, URL may be the
/// exact Workshop page, required addons identify dependency families.
pub fn pbo_cfg_patches(path: &Path) -> CfgPatchesInfo {
    use std::io::Read;
    let mut info = CfgPatchesInfo::default();
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return info,
    };
    let mut data = Vec::new();
    if file.take(4_000_000).read_to_end(&mut data).is_err() {
        return info;
    }

    let strings: Vec<String> = data
        .split(|&b| !(0x20..=0x7e).contains(&b))
        .filter(|s| s.len() >= 4)
        .map(|s| String::from_utf8_lossy(s).to_string())
        .collect();
    let text = strings.join("\n");

    // 1. CfgPatches class name: "class CfgPatches" followed by
    //    "class <name> {". This is the addon's identity.
    if let Some(idx) = text.find("CfgPatches") {
        let after_cfg = &text[idx + "CfgPatches".len()..];
        // Find the next "class " after CfgPatches
        if let Some(class_idx) = after_cfg.find("class ") {
            let after_class = &after_cfg[class_idx + "class ".len()..];
            // Take until whitespace, brace, or semicolon
            let end = after_class
                .find(|c: char| c.is_whitespace() || c == '{' || c == ';')
                .unwrap_or(after_class.len());
            if end >= 2 {
                let name = after_class[..end].trim().to_string();
                if !name.is_empty() && name != "CfgPatches" {
                    info.name = Some(name);
                }
            }
        }
    }

    // 2. author = "Name"
    for s in text.lines() {
        let s = s.trim();
        if let Some(idx) = s.find("author") {
            let after = &s[idx + 6..];
            let after = after.trim_start();
            if let Some(after) = after.strip_prefix('=') {
                let after = after.trim_start();
                if let Some(name) = extract_cfg_value(after) {
                    if !name.starts_with('$') && name.len() >= 2 {
                        info.author = Some(name);
                        break;
                    }
                }
            }
        }
    }

    // 2. url = "https://..."
    for s in text.lines() {
        let s = s.trim();
        if let Some(idx) = s.find("url") {
            // Ensure it's the field assignment, not part of a class name
            let before = s[..idx].trim();
            if before.is_empty() || before.ends_with('{') || before.ends_with(';') {
                let after = &s[idx + 3..];
                let after = after.trim_start();
                if let Some(after) = after.strip_prefix('=') {
                    let after = after.trim_start();
                    if let Some(url) = extract_cfg_value(after) {
                        if url.starts_with("http") {
                            info.url = Some(url);
                            break;
                        }
                    }
                }
            }
        }
    }

    // 3. requiredAddons[] = { "addon1", "addon2", ... }
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
                        let addon: String = addon
                            .chars()
                            .filter(|c| !c.is_whitespace() && *c != '"')
                            .collect();
                        if !addon.is_empty() {
                            info.required_addons.push(addon);
                        }
                    }
                }
            }
        }
    }

    info
}

/// Extract a quoted or bare value from a config line after `=`.
fn extract_cfg_value(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(s) = s.strip_prefix('"') {
        if let Some(end) = s.find('"') {
            return Some(s[..end].to_string());
        }
    } else {
        // Bare value: take until whitespace or semicolon
        let end = s
            .find(|c: char| c.is_whitespace() || c == ';')
            .unwrap_or(s.len());
        if end > 0 {
            return Some(s[..end].to_string());
        }
    }
    None
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_of_returns_trimmed_left_side() {
        assert_eq!(key_of("name = \"x\""), Some("name"));
        assert_eq!(key_of("  author=\"y\""), Some("author"));
        assert_eq!(key_of("no assignment"), None);
    }

    #[test]
    fn parse_mod_cpp_matches_exact_keys_only() {
        let dir = std::env::temp_dir().join("uksfta-pbo-modcpp");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mod.cpp");
        fs::write(
            &path,
            "namespace = \"wrong\"\nname = \"Right\"\nauthorName = \"wrong\"\nauthor = \"Auth\"\n",
        )
        .unwrap();
        let meta = parse_mod_cpp(&path);
        assert_eq!(meta.name, "Right");
        assert_eq!(meta.author, "Auth");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parse_meta_cpp_matches_exact_keys_only() {
        let dir = std::env::temp_dir().join("uksfta-pbo-metacpp");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("meta.cpp");
        fs::write(
            &path,
            "publishedid = \"1234567890\"\nname = \"Right\"\nnamespace = \"wrong\"\n",
        )
        .unwrap();
        let meta = parse_meta_cpp(&path);
        assert_eq!(meta.publishedid.as_deref(), Some("1234567890"));
        assert_eq!(meta.name, "Right");
        fs::remove_dir_all(&dir).unwrap();
    }

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
}
