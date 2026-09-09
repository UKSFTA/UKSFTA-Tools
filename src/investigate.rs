use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::lock::load_lock;
use crate::origin::{
    build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id, IdentityCache,
    ResolvedOrigin,
};
use crate::pbo::{
    extra_search_terms_from_prefix, get_mod_metadata, pbo_cfg_patches, pbo_config_identity,
    pbo_identity, pbo_prefix, search_term_from_prefix,
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
    /// Additional search terms derived from the full prefix path.
    /// E.g. for prefix "x\SPS\Vehicles\sps_blackhornet", the primary
    /// term is "SPS" but extras include "sps_blackhornet", "blackhornet".
    extra_terms: Vec<String>,
    pbos: Vec<String>,
    /// The full PBO prefix path (e.g. "z\ace\addons\grenades").
    /// Used for prefix path search in descriptions.
    prefix: Option<String>,
    author_handle: Option<String>,
    /// CfgPatches `author` field: full author name (e.g. "UnderSiege Productionz").
    cfg_author: Option<String>,
    /// CfgPatches `url` field: sometimes the exact Workshop page URL.
    cfg_url: Option<String>,
    /// CfgPatches class name: the addon's identity (e.g. "ffaa_data", "ade").
    /// Used as an additional search term.
    cfg_name: Option<String>,
    /// Required addons from CfgPatches: dependency mod families.
    cfg_children: Vec<String>,
}

// ── Scoring ────────────────────────────────────────────────────────────

/// Score a candidate against a group of PBOs using multiple signals.
/// Higher is better. Signals weighted by reliability:
///   search_score (Steam's own relevance): 100 pts
///   popularity (subscriptions + views):   ~45 pts
///   quality (star rating):                20 pts
///   author match (handle):                50 pts
///   URL match (CfgPatches):              100 pts (near-certain)
///   dependency children match:            +10 per required addon found
///   tag filter (Mod tag):                 10 pts
///   file_type penalty (non-mod):         -50 pts
///
/// Note: CfgPatches `author` is NOT used for scoring. In repacked mods
/// it names the repacker, not the original mod author. Matching it
/// against Workshop creator_name would be wrong.
fn score_candidate(candidate: &ScoredCandidate, group: &PboGroup) -> f64 {
    let mut score = 0.0;

    // 0. Title relevance — multiple signals combined.
    //    Substring match is strongest, word overlap catches compound
    //    terms where the full string doesn't match but words do.
    let term_lower = group.search_term.to_lowercase();
    let title_lower = candidate.title.to_lowercase();
    let title_relevant = title_lower.contains(&term_lower);

    // Word-level overlap: split term into words, check how many
    // appear in the title. "aceax" → ["aceax"] → 1.0 if "ACEAX" in title.
    // "zulu_custom" → ["zulu","custom"] → 0.5 if only "Zulu" matches.
    let word_score = word_overlap_score(&term_lower, &title_lower);

    // Word-level token split for compound terms
    let term_tokens: Vec<&str> = split_camel_or_underscore(&term_lower);
    let matching_tokens = term_tokens
        .iter()
        .filter(|t| t.len() >= 3 && title_lower.contains(**t))
        .count();
    let token_ratio = if term_tokens.is_empty() {
        0.0
    } else {
        matching_tokens as f64 / term_tokens.len() as f64
    };

    // Combine signals: substring is definitive, word overlap catches
    // compound terms, token ratio catches partial matches.
    if title_relevant {
        score += 80.0;
    } else if word_score >= 0.8 {
        // Most words match — strong signal
        score += 50.0 + word_score * 20.0;
    } else if token_ratio >= 0.5 {
        score += 40.0 * token_ratio;
    } else if word_score > 0.3 || token_ratio > 0.0 {
        // Some overlap — weak but non-zero signal
        score += 20.0 * word_score + 15.0 * token_ratio;
    } else {
        // No overlap at all — heavy penalty
        score -= 40.0;
    }

    // 1. Steam's own search_score (0-1 range)
    score += candidate.search_score * 100.0;

    // 2. Popularity (log-scaled to prevent domination by mega-mods)
    score += (candidate.subscriptions as f64 + 1.0).ln() * 15.0;
    score += (candidate.views as f64 + 1.0).ln() * 10.0;
    score += (candidate.favorited as f64 + 1.0).ln() * 5.0;

    // 3. Quality
    score += candidate.star_rating * 20.0;

    // 4. Author match — short handle from config identity (scoring signal)
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

    // 5. CfgPatches URL match — if the PBO's url field contains this
    //    candidate's Workshop ID, it's a near-certain match
    if let Some(ref cfg_url) = group.cfg_url {
        if cfg_url.contains(&candidate.id) {
            score += 100.0;
        }
    }

    // 6. Dependency children match — if candidate lists required addons
    //    that match the PBO's requiredAddons, strong structural signal
    for child in &candidate.children {
        let child_lower = child.to_lowercase();
        for required in &group.cfg_children {
            let req_lower = required.to_lowercase();
            // Match on the mod-family root: "rhsusf_c_weapons" matches
            // "rhsusf" from the candidate's children
            let req_root = req_lower.split('_').next().unwrap_or(&req_lower);
            if req_root.len() >= 3 && child_lower.contains(req_root) {
                score += 10.0;
            }
        }
    }

    // 6b. CfgPatches class name match — if the candidate title contains
    //     the addon class name or its root, strong structural signal.
    //     "ffaa_data" → "ffaa" in "FFAA MOD" → +30 pts
    if let Some(ref cfg_name) = group.cfg_name {
        let cfg_lower = cfg_name.to_lowercase();
        let cfg_root = cfg_lower.split('_').next().unwrap_or(&cfg_lower);
        if title_lower.contains(&cfg_lower) {
            score += 30.0;
        } else if cfg_root.len() >= 3 && title_lower.contains(cfg_root) {
            score += 20.0;
        }
    }

    // 7. Tag filter — bonus for mods, penalty for non-mods
    if candidate.tags.iter().any(|t| t.eq_ignore_ascii_case("Mod")) {
        score += 10.0;
    }
    if candidate.file_type != 0 {
        score -= 50.0;
    }

    // 8. Legacy dependency check — known mod families in children
    for child in &candidate.children {
        let child_lower = child.to_lowercase();
        for known in &["ace", "rhs", "cba", "tf", "tfl"] {
            if child_lower.contains(known) {
                score += 5.0;
            }
        }
    }

    // 9. Description content match — if the candidate's description
    //     contains PBO names from our group, strong confirmation signal.
    //     Some modders list their PBO contents in the description.
    let desc_lower = candidate.short_description.to_lowercase();
    let mut desc_hits = 0;
    for pbo_name in &group.pbos {
        let stem = pbo_name
            .strip_suffix(".pbo")
            .unwrap_or(pbo_name)
            .to_lowercase();
        if stem.len() >= 4 && desc_lower.contains(&stem) {
            desc_hits += 1;
        }
    }
    if desc_hits > 0 {
        score += 20.0 * desc_hits as f64;
    }

    // 10. Prefix path in description — some modders include the exact
    //     prefix path (e.g. "z\ace\addons\grenades") in their description.
    //     This is a near-certain match when found.
    if let Some(ref prefix) = group.prefix {
        let prefix_lower = prefix.to_lowercase();
        if desc_lower.contains(&prefix_lower) {
            score += 60.0;
        }
    }

    // 11. Dependency graph signal — if the PBO's required addons contain
    //     a root that matches the group's search term, the mod family is
    //     confirmed. E.g. PBO requires "rhsusf_c_weapons" and group
    //     searches for "rhsusf" → the mod is RHS.
    for required in &group.cfg_children {
        let req_lower = required.to_lowercase();
        let req_root = req_lower.split('_').next().unwrap_or(&req_lower);
        if req_root.len() >= 3 && term_lower.contains(req_root) {
            score += 25.0;
            break;
        }
    }

    score
}

/// Split a camelCase or underscore_separated string into tokens.
/// "zuluslicksters" → ["zuluslicksters"] (no split possible)
/// "zulu_custom_slicksters" → ["zulu", "custom", "slicksters"]
/// "tfl_headgear" → ["tfl", "headgear"]
fn split_camel_or_underscore(s: &str) -> Vec<&str> {
    // If underscores are present, split on them
    if s.contains('_') {
        return s.split('_').filter(|t| !t.is_empty()).collect();
    }
    // Otherwise, try camelCase split
    let mut tokens = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if c.is_uppercase() && i > start {
            let token = &s[start..i];
            if !token.is_empty() {
                tokens.push(token);
            }
            start = i;
        }
    }
    let last = &s[start..];
    if !last.is_empty() {
        tokens.push(last);
    }
    // If we only got one token, return the whole thing
    if tokens.len() <= 1 {
        vec![s]
    } else {
        tokens
    }
}

/// Character-level token overlap score between a search term and a title.
/// Breaks the search term into individual characters, checks how many
/// appear in the title. Returns a ratio (0.0 to 1.0).
///
/// "aceax" → {a,c,e,x} → "ACE3 Arsenal Extended" has a,c,e,x → 4/4 = 1.0
/// Word-level overlap score between a search term and a title.
/// Splits both into words, checks how many search-term words appear
/// in the title. More discriminative than character-level matching
/// because it avoids false positives from common letters.
///
/// "aceax" → ["aceax"] → if "ACEAX" in title → 1.0
/// "zulu_custom_slicksters" → ["zulu","custom","slicksters"] → "A2 Declassified: Fireteam Zulu" has "zulu" → 1/3 = 0.33
/// "usasoc_backpacks" → ["usasoc","backpacks"] → "121 USASOC Sniper Rifles Pack" has "usasoc" → 1/2 = 0.5
fn word_overlap_score(term_lower: &str, title_lower: &str) -> f64 {
    let term_words: Vec<&str> = split_camel_or_underscore(term_lower);
    if term_words.is_empty() {
        return 0.0;
    }

    // Split title on non-alphanumeric boundaries
    let title_words: Vec<&str> = title_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    let hits = term_words
        .iter()
        .filter(|tw| tw.len() >= 3 && title_words.iter().any(|t| *t == **tw || t.contains(*tw)))
        .count();

    hits as f64 / term_words.len() as f64
}

/// Generate multiple search variations from a single search term.
/// "sps_blackhornet" → ["sps blackhornet", "blackhornet", "sps"]
/// "ZuluCustomSlicksters" → ["zulu custom slicksters", "slicksters",
///   "zulu", "custom", "zulu custom", "custom slicksters"]
/// The idea: one term may not match the Workshop title, but a
/// substring or reordering might. We try all of them and merge.
fn search_variation_terms(term: &str) -> Vec<String> {
    let tokens = split_camel_or_underscore(term);
    let mut variations = Vec::new();

    if tokens.len() <= 1 {
        // Single token: try it as-is
        variations.push(term.to_string());
        return variations;
    }

    // 1. All tokens joined with spaces (full term)
    let full = tokens.join(" ");
    if full != term {
        variations.push(full);
    }

    // 2. Individual tokens (>= 3 chars to avoid noise)
    for t in &tokens {
        if t.len() >= 3 {
            variations.push(t.to_string());
        }
    }

    // 3. Pairs of adjacent tokens
    for w in tokens.windows(2) {
        variations.push(w.join(" "));
    }

    // 4. Last token first (reversal — "blackhornet sps")
    if tokens.len() >= 2 {
        let mut rev: Vec<&str> = tokens.iter().rev().copied().collect();
        rev.truncate(3); // cap at 3 tokens
        variations.push(rev.join(" "));
    }

    // 5. First token only (if >= 4 chars — the family root)
    if tokens[0].len() >= 4 {
        // Already added in step 2, skip
    }

    // Deduplicate while preserving order
    let mut seen = std::collections::HashSet::new();
    variations.retain(|v| seen.insert(v.clone()));
    variations
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

/// Extract a grouping key from a PBO prefix. This identifies the mod
/// family and is used to group PBOs that belong to the same Workshop
/// item.
///
/// Rules (by prefix structure):
/// - Namespaced root (`z\`, `x\`, `pz\`, `v\`, `a3\`): first 2 segments
///   (`z\aceax\addons\main` → `z\aceax`). This keeps mod families
///   together while preventing `z\` from merging all third-party mods.
/// - Non-namespaced root (`NAVSPECWARGRU2\common`): full prefix — the
///   root IS the mod identity.
/// - No prefix (bare filename): filename stem before first `_`
///   (`aceax_compat_tfl_cold` → `aceax`). This catches repacked PBOs
///   whose prefix is just the filename.
fn prefix_family(prefix: &str) -> String {
    let parts: Vec<&str> = prefix.split('\\').collect();
    if parts.len() >= 2 && parts[0].len() < 3 {
        // Namespace root: z, x, pz, v, a3, opx, mg8
        format!("{}\\{}", parts[0], parts[1])
    } else {
        prefix.to_string()
    }
}

/// Group PBOs by their mod family. The grouping key is the prefix
/// family (first 2 segments for namespaced roots, full prefix otherwise,
/// filename stem for bare names). Config identity is a scoring signal
/// only — it must not determine grouping because PBOs from the same mod
/// can have different config identities (e.g. `requiredAddons` varies
/// per PBO).
fn group_pbos_by_term(
    pbos: &[(String, Option<ResolvedOrigin>)],
    addons_dir: &Path,
) -> Vec<PboGroup> {
    let mut groups: HashMap<String, PboGroup> = HashMap::new();

    for (name, _origin) in pbos {
        let pbo_path = addons_dir.join(name);

        // Primary grouping key: prefix family
        let prefix = pbo_prefix(&pbo_path);
        let group_key = match prefix {
            Some(ref p) => prefix_family(p),
            None => {
                // No prefix — use filename stem before first `_`,
                // stripping the .pbo extension first
                let stem = name.strip_suffix(".pbo").unwrap_or(name);
                stem.split('_').next().unwrap_or(stem).to_string()
            }
        };

        // Search term for the Workshop query: derive from the group key.
        let search_term = search_term_from_prefix(&group_key);

        // Author handle from config identity (short handle, scoring signal only)
        let author_handle = pbo_config_identity(&pbo_path).filter(|t| t.len() < 6 && t.len() >= 2);

        // Full CfgPatches data: author, url, required addons.
        // Read from the first PBO in each group (free cross-reference signals).
        let cfg = pbo_cfg_patches(&pbo_path);

        let group = groups.entry(group_key.clone()).or_insert_with(|| {
            let full_prefix = prefix.clone().unwrap_or_default();
            let extra_terms = extra_search_terms_from_prefix(&full_prefix);
            PboGroup {
                search_term,
                extra_terms,
                pbos: Vec::new(),
                prefix: prefix.clone(),
                author_handle: None,
                cfg_author: cfg.author.clone(),
                cfg_url: cfg.url.clone(),
                cfg_name: cfg.name.clone(),
                cfg_children: cfg.required_addons.clone(),
            }
        });
        group.pbos.push(name.clone());
        if let Some(ref ah) = author_handle {
            if group
                .author_handle
                .as_ref()
                .is_none_or(|existing| ah.len() > existing.len())
            {
                group.author_handle = Some(ah.clone());
            }
        }
        // If the first PBO had no author/url/name, try this one
        if group.cfg_author.is_none() && cfg.author.is_some() {
            group.cfg_author = cfg.author;
        }
        if group.cfg_url.is_none() && cfg.url.is_some() {
            group.cfg_url = cfg.url;
        }
        if group.cfg_name.is_none() && cfg.name.is_some() {
            group.cfg_name = cfg.name;
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
        "https://steamcommunity.com/workshop/browse/?appid=107410&searchtext={}&requiredtags[]=Mod",
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

// ── Changelog scraping ────────────────────────────────────────────────

/// Scrape the Workshop changelog page for a mod and extract mod-name
/// references. The changelog often names source mods that a pack
/// repacked, e.g. "Updated ACE3 to 3.16.0" or "Added FFAA MOD".
fn scrape_changelog(workshop_id: &str, client: &reqwest::blocking::Client) -> Vec<String> {
    let url = format!(
        "https://steamcommunity.com/sharedfiles/filedetails/changelog/{}",
        workshop_id
    );
    let html = match client.get(&url).send().ok().and_then(|r| r.text().ok()) {
        Some(h) => h,
        None => return Vec::new(),
    };

    let mut names = Vec::new();
    // Changelog entries are in <div class="entry"> blocks. Extract text
    // content and look for known mod name patterns.
    for entry in html.split("<div class=\"entry\">").skip(1) {
        let text_end = entry.find("</div>").unwrap_or(entry.len());
        let text = &entry[..text_end];
        // Strip HTML tags to get plain text
        let mut in_tag = false;
        let plain: String = text
            .chars()
            .filter(|&c| {
                if c == '<' {
                    in_tag = true;
                    false
                } else if c == '>' {
                    in_tag = false;
                    false
                } else {
                    !in_tag
                }
            })
            .collect();
        let plain = plain.trim();
        if plain.len() > 5 {
            names.push(plain.to_string());
        }
    }
    names
}

// ── Bikey detection ───────────────────────────────────────────────────

/// Scan for .bikey files in the mod pack's parent directory. The key
/// name often identifies the author team (e.g. "TFB.bistkeys",
/// "ZSquadron.bistkeys"). Returns a list of key base names.
fn scan_bikey_names(addons_dir: &Path) -> Vec<String> {
    // Bikey files are typically in the parent of the addons/ folder
    // (i.e. the Workshop item root: workshop/content/107410/<id>/keys/)
    let pack_root = addons_dir.parent().unwrap_or(addons_dir);
    let keys_dir = pack_root.join("keys");
    let mut names = Vec::new();

    if let Ok(entries) = fs::read_dir(&keys_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".bikey") || name.ends_with(".bisign") {
                let base = name
                    .rsplit_once('.')
                    .map(|(b, _)| b.to_string())
                    .unwrap_or(name);
                if !names.contains(&base) {
                    names.push(base);
                }
            }
        }
    }
    names
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

        // Scan for bikey files — author team signal
        let bikey_names = scan_bikey_names(addons_dir);
        if !bikey_names.is_empty() {
            println!("  Signing keys found: {}", bikey_names.join(", "));
        }

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

            // Generate multiple search variations to cast a wider net.
            // "sps_blackhornet" → ["sps blackhornet", "blackhornet", "sps"]
            // Plus extra terms from the full prefix path.
            let mut variations = search_variation_terms(term);
            for extra in &group.extra_terms {
                if !variations.contains(extra) {
                    variations.push(extra.clone());
                }
            }
            // Add CfgPatches class name as a search term.
            // "ffaa_data" → search "ffaa" (the mod family)
            if let Some(ref cfg_name) = group.cfg_name {
                let cfg_root = cfg_name.split('_').next().unwrap_or(cfg_name);
                if cfg_root.len() >= 3 && !variations.contains(&cfg_root.to_string()) {
                    variations.push(cfg_root.to_string());
                }
                if !variations.contains(cfg_name) {
                    variations.push(cfg_name.clone());
                }
            }
            // Add author handle as a search term when distinctive.
            // "TFB" → search "TFB" — Workshop titles often include author names.
            if let Some(ref author) = group.author_handle {
                if !variations.contains(author) {
                    variations.push(author.clone());
                }
            }
            // Add required addon roots as search terms.
            // If PBO requires "rhsusf_c_weapons", search "rhsusf" —
            // the dependency's mod family often appears in the title.
            for required in &group.cfg_children {
                let req_root = required.split('_').next().unwrap_or(required);
                if req_root.len() >= 3 && !variations.contains(&req_root.to_string()) {
                    variations.push(req_root.to_string());
                }
            }

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
                // Search each variation and merge results. Keep the
                // highest search_score for each candidate ID.
                let mut merged: std::collections::HashMap<String, ScoredCandidate> =
                    std::collections::HashMap::new();
                let mut any_from_cache = false;

                for variation in &variations {
                    if searched.contains(variation) {
                        // Already searched this variation in another group
                        if let Some(cached) = cache.get(variation) {
                            for (id, title) in cached {
                                let c = cache_to_candidate(id, title);
                                if let Some(existing) = merged.get(id) {
                                    if c.search_score > existing.search_score {
                                        merged.insert(id.clone(), c);
                                    }
                                } else {
                                    merged.insert(id.clone(), c);
                                }
                            }
                            any_from_cache = true;
                        }
                        continue;
                    }

                    let rich = search_workshop_api(variation).unwrap_or_else(|| {
                        let scraped = search_workshop(variation);
                        if scraped.is_empty() {
                            let ids: Vec<String> = scraped.iter().map(|c| c.id.clone()).collect();
                            batch_workshop_details(&ids, &api_client)
                        } else {
                            scraped
                        }
                    });

                    // Merge into results, keeping the best score
                    for c in rich {
                        if let Some(existing) = merged.get(&c.id) {
                            if c.search_score > existing.search_score {
                                merged.insert(c.id.clone(), c);
                            }
                        } else {
                            merged.insert(c.id.clone(), c);
                        }
                    }

                    // Cache this variation's results
                    let cached: Vec<(String, String)> =
                        merged.values().map(candidate_to_cache).collect();
                    cache.insert(variation.clone(), cached);
                    save_identity_cache(&cache);
                    searched.insert(variation.clone());

                    // Rate-limit only real Workshop page hits
                    if !any_from_cache {
                        std::thread::sleep(std::time::Duration::from_millis(1100));
                    }
                }

                let candidates: Vec<ScoredCandidate> = merged.into_values().collect();
                // Cache under the primary term too
                if !candidates.is_empty() {
                    let cached: Vec<(String, String)> =
                        candidates.iter().map(candidate_to_cache).collect();
                    cache.insert(term.clone(), cached);
                    save_identity_cache(&cache);
                }
                (candidates, any_from_cache)
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

                // Check title relevance of the top candidate.
                // If the search term doesn't appear anywhere in the
                // top candidate's title, the match is probably noise.
                let term_lower = term.to_lowercase();
                let top_title_lower = scored
                    .first()
                    .map(|(_, c)| c.title.to_lowercase())
                    .unwrap_or_default();
                let title_match = top_title_lower.contains(&term_lower);

                // Character-level token overlap for the top candidate
                let top_word_score = word_overlap_score(&term_lower, &top_title_lower);

                // Also check word-level token relevance
                let term_tokens = split_camel_or_underscore(&term_lower);
                let top_token_hits = term_tokens
                    .iter()
                    .filter(|t| t.len() >= 3 && top_title_lower.contains(**t))
                    .count();
                let token_ratio = if term_tokens.is_empty() {
                    0.0
                } else {
                    top_token_hits as f64 / term_tokens.len() as f64
                };

                let weak_match = scored.first().map(|(s, _)| *s < 30.0).unwrap_or(true)
                    || (!title_match && top_word_score < 0.5 && token_ratio < 0.5);

                // Show group summary with cross-reference signals
                // Note: cfg_author is NOT shown — in repacked mods it names
                // the repacker, not the original mod author.
                let mut meta = Vec::new();
                if let Some(ref url) = group.cfg_url {
                    // Only show Workshop-like URLs (not Twitch, GitHub, etc.)
                    if url.contains("steamcommunity.com/sharedfiles") {
                        meta.push(format!("url={}", url));
                    }
                }
                if !group.cfg_children.is_empty() {
                    meta.push(format!("deps=[{}]", group.cfg_children.join(", ")));
                }
                let meta_str = if meta.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", meta.join(", "))
                };

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

                if weak_match {
                    // Weak match: search term doesn't appear in the
                    // top candidate's title. Show "no confident match"
                    // with the best candidate for reference.
                    println!(
                        "  [group: {}] (search \"{}\"): {} candidate(s){} — no confident match (best: {})",
                        group.pbos.join(", "),
                        term,
                        candidates.len(),
                        meta_str,
                        top.first().map(|s| s.as_str()).unwrap_or("none")
                    );
                } else {
                    println!(
                        "  [group: {}] (search \"{}\"): {} candidate(s){} — {}",
                        group.pbos.join(", "),
                        term,
                        candidates.len(),
                        meta_str,
                        top.join(", ")
                    );
                }

                // Changelog cross-reference: if the top candidate has a
                // changelog, check if it names mod families from our group.
                // This catches cases where a pack lists its source mods.
                if !from_cache && !scored.is_empty() {
                    if let Some((_score, top_candidate)) = scored.first() {
                        let notes = scrape_changelog(&top_candidate.id, &api_client);
                        for note in &notes {
                            let note_lower = note.to_lowercase();
                            // Check if any PBO name or group key appears
                            // in the changelog text
                            for pbo_name in &group.pbos {
                                let stem = pbo_name
                                    .rsplit_once('.')
                                    .map(|(s, _)| s)
                                    .unwrap_or(pbo_name);
                                let stem_lower = stem.to_lowercase();
                                if stem_lower.len() >= 4 && note_lower.contains(&stem_lower) {
                                    println!(
                                        "    changelog hit: \"{}\" matches {}",
                                        &note[..note.len().min(100)],
                                        pbo_name
                                    );
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            // Rate-limiting is handled inside the variation loop above.
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
