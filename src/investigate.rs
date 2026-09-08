use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::lock::load_lock;
use crate::origin::{
    build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id, IdentityCache,
    ResolvedOrigin,
};
use crate::pbo::{
    get_mod_metadata, pbo_config_identity, pbo_identity, pbo_prefix, search_term_from_prefix,
};
use crate::steam::find_all_workshop_caches;
use crate::util::{urlencode, workshop_url};

/// Print the resolved identity inventory from the local cache.
/// For each untracked PBO in addons/, derive its search term (same logic
/// as the online search) and show the cached candidate mods, if any.
/// No network: this reads only .uksfta/identities.json.
pub fn investigate_report() {
    let addons_dir = Path::new("addons");
    if !addons_dir.exists() {
        eprintln!("No addons/ directory");
        return;
    }

    let cache = load_identity_cache();
    if cache.is_empty() {
        println!("No cached identities found. Run 'uksfta investigate --online' first.");
        return;
    }

    println!("Resolved identity inventory (from local cache):");
    let mut shown = 0;
    if let Ok(entries) = fs::read_dir(addons_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "pbo").unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                let term = pbo_config_identity(&path)
                    .or_else(|| pbo_identity(&path))
                    .or_else(|| pbo_prefix(&path))
                    .unwrap_or_else(|| name.clone());
                let term = search_term_from_prefix(&term);
                if let Some(hits) = cache.get(&term) {
                    if !hits.is_empty() {
                        let best = &hits[0];
                        println!(
                            "  {:<45} -> {} ({}) [term \"{}\"]",
                            name, best.1, best.0, term
                        );
                        shown += 1;
                    }
                }
            }
        }
    }
    if shown == 0 {
        println!("  (no cached identities match the PBOs in addons/)");
    } else {
        println!("\n{} PBO(s) have cached identity candidates.", shown);
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

/// Trace the Workshop origin of untracked PBOs in addons/.
pub fn investigate(all: bool, online: bool) {
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
