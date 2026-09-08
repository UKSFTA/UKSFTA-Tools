use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::util::{extract_quoted, extract_quoted_tokens};

pub const STEAM_APP_ID: &str = "107410";

// --- VDF Metadata ---

#[derive(Debug, Clone, Default)]
pub struct WorkshopItem {
    pub size: u64,
    pub time_updated: u64,
}

pub fn parse_acf(content: &str) -> HashMap<String, WorkshopItem> {
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

pub fn find_steam_library_folders() -> Vec<PathBuf> {
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

pub fn find_all_workshop_caches() -> Vec<PathBuf> {
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
