//! Candidate scoring against a PBO group.

use super::model::{PboGroup, ScoredCandidate};
use super::text::{split_camel_or_underscore, title_contains_term, word_overlap_score};

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
pub(crate) fn score_candidate(candidate: &ScoredCandidate, group: &PboGroup) -> f64 {
    let mut score = 0.0;

    // 0. Title relevance — multiple signals combined.
    //    Substring match is strongest, word overlap catches compound
    //    terms where the full string doesn't match but words do.
    let term_lower = group.search_term.to_lowercase();
    let title_lower = candidate.title.to_lowercase();
    // Short terms must match as a whole word, longer terms as substring
    let title_relevant = title_contains_term(&title_lower, &term_lower);

    // Word-level overlap: split term into words, check how many
    // appear in the title. "aceax" → ["aceax"] → 1.0 if "ACEAX" in title.
    // "zulu_custom" → ["zulu","custom"] → 0.5 if only "Zulu" matches.
    let word_score = word_overlap_score(&term_lower, &title_lower);

    // Word-level token split for compound terms
    let term_tokens: Vec<&str> = split_camel_or_underscore(&term_lower);
    let matching_tokens = term_tokens
        .iter()
        .filter(|t| t.len() >= 3 && title_contains_term(&title_lower, t))
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

    // 9b. PBO-name word match — check the candidate title against every
    //     PBO stem word in the group, not just the search term. Catches
    //     cases where the derived search term is a generic author prefix
    //     ("JAS") but the PBO name is descriptive ("NVG_Parts" → "nvg"
    //     suffix-matches "GPNVG18"). Words equal to the search term are
    //     skipped — title relevance already scored them.
    let mut pbo_name_hits = 0;
    for pbo_name in &group.pbos {
        // Split BEFORE lowercasing: camelCase boundaries ("FranksMarkers"
        // → ["Franks", "Markers"]) are lost if we lowercase first.
        let stem = pbo_name.strip_suffix(".pbo").unwrap_or(pbo_name);
        for word in split_camel_or_underscore(stem) {
            let word = word.to_lowercase();
            if word.len() >= 3 && word != term_lower && title_contains_term(&title_lower, &word) {
                pbo_name_hits += 1;
                break;
            }
        }
    }
    if pbo_name_hits > 0 {
        score += 25.0 * pbo_name_hits as f64;
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
