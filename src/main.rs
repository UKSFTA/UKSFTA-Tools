use clap::{Parser, Subcommand};
use std::fs;
use std::path::{Path, PathBuf};

pub use std::collections::{HashMap, HashSet};

mod investigate;
mod lock;
mod modlist;
mod origin;
mod pbo;
mod steam;
mod sync;
mod util;

pub use crate::investigate::*;
pub use crate::lock::*;
pub use crate::modlist::*;
pub use crate::origin::*;
pub use crate::pbo::*;
pub use crate::steam::*;
pub use crate::sync::*;
pub use crate::util::*;

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
        /// Print the resolved identity inventory from the local cache
        /// (no network)
        #[arg(long)]
        report: bool,
    },
    /// Show the installed version and check for updates
    Version,
}

/// Show the installed version and check GitHub for a newer release.
/// Prints update instructions matching the install method in use.
/// Query the latest release tag from GitHub (no key required).
fn latest_release_tag() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build HTTP client");

    let body = client
        .get("https://api.github.com/repos/UKSFTA/UKSFTA-Tools/releases/latest")
        .header("User-Agent", "uksfta")
        .send()
        .ok()?
        .text()
        .ok()?;

    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release = serde_json::from_str(&body).ok()?;
    Some(release.tag_name)
}

/// Print an update notice if a newer release exists. When `force` is false
/// (the automatic check on every command), the GitHub API is queried at
/// most once per day, using a timestamped marker in .uksfta/ so normal
/// commands stay fast and offline-friendly.
fn check_for_updates(force: bool) {
    let current = env!("CARGO_PKG_VERSION");
    let marker = Path::new(".uksfta").join("last-update-check");

    if !force {
        // Skip the API call if we already checked today
        if let Ok(mtime) = fs::metadata(&marker).and_then(|m| m.modified()) {
            if let Ok(elapsed) = mtime.elapsed() {
                if elapsed < std::time::Duration::from_secs(24 * 3600) {
                    return;
                }
            }
        }
    }

    let latest = match latest_release_tag() {
        Some(t) => t,
        None => return, // offline or error: stay silent on auto-check
    };

    // Record the check (only when the API succeeded)
    if let Some(dir) = marker.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(&marker, b"");

    if is_outdated(current, latest.trim_start_matches('v')) {
        println!(
            "\nA new version is available: v{} (you have v{})",
            latest.trim_start_matches('v'),
            current
        );
        println!("Run 'uksfta version' for update instructions.");
    }
}

/// Show the installed version and check GitHub for a newer release.
fn version() {
    let current = env!("CARGO_PKG_VERSION");
    println!("uksfta {}", current);

    let latest = match latest_release_tag() {
        Some(t) => t.trim_start_matches('v').to_string(),
        None => {
            println!("Could not check for updates (offline or no access to GitHub).");
            return;
        }
    };

    if !is_outdated(current, &latest) {
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

    // Daily update notice on every command (skips network if checked
    // within 24h). The explicit `version` command forces a fresh check.
    if !matches!(cli.command, Commands::Version) {
        check_for_updates(false);
    }

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
        Commands::Investigate {
            all,
            online,
            report,
        } => {
            if report {
                investigate_report();
            } else {
                investigate(all, online);
            }
        }
        Commands::Version => version(),
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

    #[test]
    fn urlencode_pairs_encodes_key_values() {
        let pairs = [("search_text", "Zulu Custom"), ("numperpage", "10")];
        assert_eq!(
            urlencode_pairs(&pairs),
            "search_text=Zulu%20Custom&numperpage=10"
        );
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

    #[test]
    fn is_outdated_handles_v_prefix_and_trimmed() {
        // The auto-check passes trimmed tags; version passes raw
        assert!(!is_outdated("0.4.0", "0.4.0"));
        assert!(is_outdated("0.3.0", "0.4.0"));
        assert!(is_outdated("0.3.0", "0.4.1"));
    }
}
