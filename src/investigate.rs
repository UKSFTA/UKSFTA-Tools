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

// ── Data structures ────────────────────────────────────────────────────

/// A Workshop search candidate with all ranking signals.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // fields stored for completeness; scored/displayed selectively
struct ScoredCandidate {
    id: String,
    title: String,
    search_score: f64,
    subscriptions: u64,
    views: u64,
    favorited: u64,
    star_rating: f64,
    total_votes: u32,
    tags: Vec<String>,
    creator: String,
    creator_name: String,
    time_updated: u64,
    short_description: String,
    children: Vec<String>,
    file_type: u32,
}

/// A group of PBOs that share the same search term.
/// Arma groups PBOs by `@ModName/` folder — all PBOs in one folder
/// load as one mod. Grouping by search term means we search once per
/// mod family instead of once per PBO.
struct PboGroup {
    search_term: String,
    pbos: Vec<String>,
    author_handle: Option<String>,
}

// ── Scoring ────────────────────────────────────────────────────────────

/// Score a candidate against a group of PBOs using multiple signals.
/// Higher is better. Signals weighted by reliability:
///   search_score (Steam's own relevance): 100 pts
///   popularity (subscriptions + views):   ~45 pts
///   quality (star rating):                20 pts
///   author match:                         50 pts
///   tag filter (Mod tag):                 10 pts
///   file_type penalty (non-mod):         -50 pts
fn score_candidate(candidate: &ScoredCandidate, group: &PboGroup) -> f64 {
    let mut score = 0.0;

    // 1. Steam's own search_score (0-1 range)
    score += candidate.search_score * 100.0;

    // 2. Popularity (log-scaled to prevent domination by mega-mods)
    score += (candidate.subscriptions as f64 + 1.0).ln() * 15.0;
    score += (candidate.views as f64 + 1.0).ln() * 10.0;
    score += (candidate.favorited as f64 + 1.0).ln() * 5.0;

    // 3. Quality
    score += candidate.star_rating * 20.0;

    // 4. Author match — strong signal when author handle present
    if let Some(ref author) = group.author_handle {
        let author_lower = author.to_lowercase();
        if candidate
            .creator_name
            .to_lowercase()
            .contains(&author_lower)
        {
            score += 50.0;
        }
    }

    // 5. Tag filter — bonus for mods, penalty for non-mods
    if candidate.tags.iter().any(|t| t.eq_ignore_ascii_case("Mod")) {
        score += 10.0;
    }
    if candidate.file_type != 0 {
        score -= 50.0;
    }

    // 6. Dependency verification — if candidate lists children that
    //    match known mod families (ace, rhs, cba), boost score
    for child in &candidate.children {
        let child_lower = child.to_lowercase();
        for known in &["ace", "rhs", "cba", "tf", "tfl"] {
            if child_lower.contains(known) {
                score += 5.0;
            }
        }
    }

    score
}

/// Convert a ScoredCandidate to the cache format (id, title).
fn candidate_to_cache(c: &ScoredCandidate) -> (String, String) {
    (c.id.clone(), c.title.clone())
}

/// Convert a cached (id, title) pair to a minimal ScoredCandidate.
fn cache_to_candidate(id: &str, title: &str) -> ScoredCandidate {
    ScoredCandidate {
        id: id.to_string(),
        title: title.to_string(),
        search_score: 0.0, // cached entries have no score data
        ..Default::default()
    }
}

// ── PBO grouping ───────────────────────────────────────────────────────

/// Group PBOs by their search term. All PBOs with the same prefix root
/// are grouped together — this matches how Arma loads them (one mod
/// family = one search). The best author handle from the group is used
/// for scoring.
fn group_pbos_by_term(
    pbos: &[(String, Option<ResolvedOrigin>)],
    addons_dir: &Path,
) -> Vec<PboGroup> {
    let mut groups: HashMap<String, PboGroup> = HashMap::new();

    for (name, _origin) in pbos {
        let pbo_path = addons_dir.join(name);
        // Extract the search term using the same priority chain as before:
        // 1. requiredAddons root from plain-text config
        // 2. string-table token from config
        // 3. short author handle from config
        // 4. header prefix
        let term = pbo_config_identity(&pbo_path)
            .or_else(|| pbo_identity(&pbo_path))
            .or_else(|| pbo_prefix(&pbo_path))
            .unwrap_or_else(|| name.clone());
        let term = search_term_from_prefix(&term);

        // Extract author handle for scoring (short tokens like DANZ, TFB)
        let author_handle = pbo_config_identity(&pbo_path).filter(|t| t.len() < 6 && t.len() >= 2);

        let group = groups.entry(term.clone()).or_insert_with(|| PboGroup {
            search_term: term,
            pbos: Vec::new(),
            author_handle: None,
        });
        group.pbos.push(name.clone());
        // Keep the best author handle (longest = most specific)
        if let Some(ref ah) = author_handle {
            if group
                .author_handle
                .as_ref()
                .is_none_or(|existing| ah.len() > existing.len())
            {
                group.author_handle = Some(ah.clone());
            }
        }
    }

    groups.into_values().collect()
}

// ── Cache ──────────────────────────────────────────────────────────────

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

// ── Workshop search (keyless scrape) ───────────────────────────────────

/// Search the Steam Workshop browse page for a query and return candidate
/// ScoredCandidates. Parses the SSR JSON blob for rich data (tags,
/// subscriptions, creator name, etc.) instead of just extracting IDs.
/// Returns an empty vec on network or parse failure.
fn search_workshop(query: &str) -> Vec<ScoredCandidate> {
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

    // Try to extract the SSR JSON blob first (rich data).
    if let Some(candidates) = parse_ssr_blob(&html) {
        return candidates;
    }

    // Fallback: regex ID extraction (original approach, no ranking data).
    // Enrich via batch details so titles and stats are available.
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
    if ids.is_empty() {
        return Vec::new();
    }
    batch_workshop_details(&ids, &client)
}

/// Parse the `window.SSR.renderContext` JSON blob from the Workshop
/// browse page. This blob contains 30 items per page with full metadata
/// (tags, subscriptions, creator name, star rating, etc.) that the
/// regex approach misses entirely.
fn parse_ssr_blob(html: &str) -> Option<Vec<ScoredCandidate>> {
    // The blob is embedded as: window.SSR.renderContext=JSON.parse("...")
    let prefix = "window.SSR.renderContext=JSON.parse(\"";
    let start = html.find(prefix)? + prefix.len();
    let end = html[start..].find("\");")? + start;
    let escaped = &html[start..end];

    // Unescape the JSON string: \" -> ", \\ -> \
    let json_str = escaped.replace("\\\"", "\"").replace("\\\\", "\\");

    #[derive(serde::Deserialize)]
    struct SsrContext {
        #[serde(rename = "workshop_browse")]
        workshop_browse: Option<WorkshopBrowse>,
    }
    #[derive(serde::Deserialize)]
    struct WorkshopBrowse {
        results: Vec<SsrItem>,
    }
    #[derive(serde::Deserialize)]
    struct SsrItem {
        #[serde(default)]
        publishedfileid: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        subscriptions: u64,
        #[serde(default)]
        views: u64,
        #[serde(default)]
        favorited: u64,
        #[serde(default)]
        star_rating: f64,
        #[serde(default)]
        total_votes: u32,
        #[serde(default)]
        tags: Vec<SsrTag>,
        #[serde(default)]
        creator: String,
        #[serde(default)]
        short_description: String,
        #[serde(default)]
        time_updated: u64,
        #[serde(default)]
        file_type: u32,
        #[serde(default)]
        creator_player_link_details: Option<CreatorDetails>,
        #[serde(default)]
        children: Option<Vec<SsrChild>>,
    }
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct SsrTag {
        #[serde(default)]
        tag: String,
        #[serde(default)]
        display_name: String,
    }
    #[derive(serde::Deserialize)]
    struct CreatorDetails {
        #[serde(default)]
        persona_name: String,
    }
    #[derive(serde::Deserialize)]
    struct SsrChild {
        #[serde(default)]
        publishedfileid: String,
    }

    let ctx: SsrContext = serde_json::from_str(&json_str).ok()?;
    let browse = ctx.workshop_browse?;
    let mut results = Vec::new();
    for item in browse.results {
        results.push(ScoredCandidate {
            id: item.publishedfileid,
            title: item.title,
            search_score: 0.0, // SSR blob has no search_score field
            subscriptions: item.subscriptions,
            views: item.views,
            favorited: item.favorited,
            star_rating: item.star_rating,
            total_votes: item.total_votes,
            tags: item.tags.into_iter().map(|t| t.display_name).collect(),
            creator: item.creator.clone(),
            creator_name: item
                .creator_player_link_details
                .map(|d| d.persona_name)
                .unwrap_or_default(),
            time_updated: item.time_updated,
            short_description: item.short_description,
            children: item
                .children
                .unwrap_or_default()
                .into_iter()
                .map(|c| c.publishedfileid)
                .collect(),
            file_type: item.file_type,
        });
    }
    Some(results)
}

// ── Workshop search (keyed QueryFiles API) ─────────────────────────────

/// Batch-confirm a list of Workshop item IDs via the keyless
/// GetPublishedFileDetails API. Returns ScoredCandidates for the items
/// that exist. Empty on network or parse failure.
fn batch_workshop_details(
    ids: &[String],
    client: &reqwest::blocking::Client,
) -> Vec<ScoredCandidate> {
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
        #[serde(default)]
        subscriptions: u64,
        #[serde(default)]
        views: u64,
        #[serde(default)]
        favorited: u64,
        #[serde(default)]
        time_updated: u64,
        #[serde(default)]
        tags: Vec<TagDetail>,
        #[serde(default)]
        creator: String,
    }
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct TagDetail {
        #[serde(default)]
        tag: String,
        #[serde(default)]
        display_name: String,
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
        .map(|d| ScoredCandidate {
            id: d.publishedfileid,
            title: d.title,
            subscriptions: d.subscriptions,
            views: d.views,
            favorited: d.favorited,
            time_updated: d.time_updated,
            tags: d.tags.into_iter().map(|t| t.display_name).collect(),
            creator: d.creator,
            ..Default::default()
        })
        .collect()
}

/// Search the Workshop via the keyed IPublishedFileService/QueryFiles API.
/// Requires STEAM_API_KEY in the environment. Returns ScoredCandidates
/// with full ranking signals. None when the key is absent or the API
/// call fails, so callers can fall back to the scrape.
fn search_workshop_api(query: &str) -> Option<Vec<ScoredCandidate>> {
    let key = std::env::var("STEAM_API_KEY").ok()?;
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("Failed to build HTTP client");

    let params = [
        ("key", key.as_str()),
        ("format", "json"),
        ("appid", "107410"),
        ("numperpage", "10"),
        ("query_type", "12"), // k_PublishedFileQueryType_RankedByTextSearch
        ("return_short_description", "1"),
        ("return_tags", "1"),
        ("return_children", "1"),
        ("return_metadata", "1"),
        ("search_text", query),
    ];
    let url = format!(
        "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/?{}",
        urlencode_pairs(&params)
    );

    let body = client.get(&url).send().ok()?.text().ok()?;

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
        #[serde(default)]
        title: String,
        #[serde(default)]
        subscriptions: u64,
        #[serde(default)]
        views: u64,
        #[serde(default)]
        favorited: u64,
        #[serde(default)]
        time_updated: u64,
        #[serde(default)]
        tags: Vec<TagDetail>,
        #[serde(default)]
        creator: String,
        #[serde(default)]
        short_description: String,
        #[serde(default)]
        children: Vec<ChildDetail>,
    }
    #[derive(serde::Deserialize)]
    struct TagDetail {
        #[serde(default)]
        #[allow(dead_code)]
        tag: String,
        #[serde(default)]
        display_name: String,
    }
    #[derive(serde::Deserialize)]
    struct ChildDetail {
        #[serde(default)]
        publishedfileid: String,
    }

    let parsed: ApiResponse = serde_json::from_str(&body).ok()?;
    Some(
        parsed
            .response
            .publishedfiledetails
            .into_iter()
            .map(|d| ScoredCandidate {
                id: d.publishedfileid,
                title: d.title,
                subscriptions: d.subscriptions,
                views: d.views,
                favorited: d.favorited,
                time_updated: d.time_updated,
                tags: d.tags.into_iter().map(|t| t.display_name).collect(),
                creator: d.creator.clone(),
                creator_name: d.creator.clone(), // QueryFiles doesn't return display name
                short_description: d.short_description,
                children: d.children.into_iter().map(|c| c.publishedfileid).collect(),
                ..Default::default()
            })
            .collect(),
    )
}

/// Percent-encode a list of (key, value) pairs for a query string.
pub fn urlencode_pairs(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

// ── Main investigate flow ──────────────────────────────────────────────

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
    // investigating a pack's own contents), exclude it as a candidate.
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

    let mut results: Vec<(String, Option<ResolvedOrigin>)> = Vec::new();

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

        // ── Grouped Workshop search ────────────────────────────────
        // Group PBOs by search term so we search once per mod family
        // instead of once per PBO. This eliminates duplicate searches
        // and cross-match noise (e.g. 3 PBOs from one mod no longer
        // search independently and potentially match 3 different mods).
        let searchable: Vec<(String, Option<ResolvedOrigin>)> = results
            .iter()
            .filter(|(_, origin)| match origin {
                None => true,
                Some(o) => o.is_pack,
            })
            .cloned()
            .collect();

        if searchable.is_empty() {
            return;
        }

        println!(
            "\nSearching Workshop for {} PBO(s) whose origin is unknown or a pack...",
            searchable.len()
        );

        let groups = group_pbos_by_term(&searchable, addons_dir);
        let mut cache = load_identity_cache();
        let api_client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .expect("Failed to build HTTP client");
        let mut searched = HashSet::new();

        for group in &groups {
            let term = &group.search_term;
            if searched.contains(term) {
                continue;
            }
            searched.insert(term.clone());

            // Check the local cache first; only hit the Workshop for
            // terms we have not already searched.
            let (candidates, from_cache): (Vec<ScoredCandidate>, bool) = if let Some(cached) =
                cache.get(term)
            {
                println!(
                    "  [group: {}] (search \"{}\"): from cache",
                    group.pbos.join(", "),
                    term
                );
                (
                    cached
                        .iter()
                        .map(|(id, title)| cache_to_candidate(id, title))
                        .collect(),
                    true,
                )
            } else {
                // Prefer the keyed QueryFiles API when STEAM_API_KEY
                // is set; fall back to the keyless browse-page scrape.
                let rich = search_workshop_api(term).unwrap_or_else(|| {
                    let scraped = search_workshop(term);
                    if scraped.is_empty() {
                        // Scrape returned nothing useful — batch-confirm IDs
                        let ids: Vec<String> = scraped.iter().map(|c| c.id.clone()).collect();
                        batch_workshop_details(&ids, &api_client)
                    } else {
                        scraped
                    }
                });
                // Convert to cache format and persist
                let cached: Vec<(String, String)> = rich.iter().map(candidate_to_cache).collect();
                cache.insert(term.clone(), cached);
                save_identity_cache(&cache);
                (rich, false)
            };

            if candidates.is_empty() {
                for pbo in &group.pbos {
                    println!("  {}: no candidates for \"{}\"", pbo, term);
                }
            } else {
                // Score and rank candidates for this group
                let mut scored: Vec<(f64, &ScoredCandidate)> = candidates
                    .iter()
                    .map(|c| (score_candidate(c, group), c))
                    .collect();
                scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

                // Show group summary
                let top = scored
                    .iter()
                    .take(5)
                    .map(|(score, c)| {
                        let stats = if c.subscriptions > 0 || c.views > 0 {
                            format!(
                                " [score={:.0}, {} subs, {} views]",
                                score, c.subscriptions, c.views
                            )
                        } else {
                            format!(" [score={:.0}]", score)
                        };
                        format!("{} ({}){}", c.title, c.id, stats)
                    })
                    .collect::<Vec<_>>();

                println!(
                    "  [group: {}] (search \"{}\"): {} candidate(s) — {}",
                    group.pbos.join(", "),
                    term,
                    candidates.len(),
                    top.join(", ")
                );
            }
            // Rate-limit only real Workshop page hits, not cache reads.
            if !from_cache {
                std::thread::sleep(std::time::Duration::from_millis(1100));
            }
        }
    }
}

// ── Workshop visibility check ──────────────────────────────────────────

/// Query the Steam Workshop API for each investigated mod's visibility.
/// Keyless: ISteamRemoteStorage/GetPublishedFileDetails needs no API key.
fn check_workshop_visibility(results: Vec<(String, Option<ResolvedOrigin>)>) {
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
