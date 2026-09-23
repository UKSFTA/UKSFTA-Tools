//! One PBO group's Workshop search, scoring, and printing.

use std::collections::HashSet;

use super::api::{batch_workshop_details, search_workshop_api};
use super::cache::save_identity_cache;
use super::changelog::scrape_changelog;
use super::model::{PboGroup, ScoredCandidate};
use super::scoring::score_candidate;
use super::search::search_workshop;
use super::text::{
    human_count, search_variation_terms, split_camel_or_underscore, title_contains_term,
    word_overlap_score,
};
use crate::origin::IdentityCache;
use crate::util::{paint, sanitize_for_terminal, truncate_chars, C_DIM, C_GREEN, C_RED, C_YELLOW};

/// Per-run counters for the online search summary.
#[derive(Default)]
pub(crate) struct GroupCounters {
    pub(crate) confident: usize,
    pub(crate) weak: usize,
    pub(crate) no_candidate: usize,
    pub(crate) cached: usize,
}

/// Search, score, and print one PBO group. A group whose search term was
/// already handled in this run is skipped.
pub(crate) fn search_group(
    group: &PboGroup,
    progress: &str,
    use_colour: bool,
    api_client: &reqwest::blocking::Client,
    cache: &mut IdentityCache<ScoredCandidate>,
    searched: &mut HashSet<String>,
    counters: &mut GroupCounters,
) {
    let term = &group.search_term;
    if searched.contains(term) {
        return;
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
            "  {} {} [group: {}] (search \"{}\"): from cache",
            paint(use_colour, C_DIM, progress),
            paint(use_colour, C_DIM, "[cached]"),
            group.pbos.join(", "),
            term
        );
        counters.cached += 1;
        (cached.clone(), true)
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
                    for c in cached {
                        if let Some(existing) = merged.get(&c.id) {
                            if c.search_score > existing.search_score {
                                merged.insert(c.id.clone(), c.clone());
                            }
                        } else {
                            merged.insert(c.id.clone(), c.clone());
                        }
                    }
                    any_from_cache = true;
                }
                continue;
            }

            let rich = match search_workshop_api(variation) {
                Some(candidates) => candidates,
                None => {
                    let scraped = match search_workshop(variation) {
                        Ok(scraped) => scraped,
                        Err(e) => {
                            eprintln!(
                                    "Warning: Workshop search for \"{}\" failed ({}), will retry next run",
                                    variation, e
                                );
                            continue;
                        }
                    };
                    if scraped.is_empty() {
                        let ids: Vec<String> = scraped.iter().map(|c| c.id.clone()).collect();
                        match batch_workshop_details(&ids, api_client) {
                            Ok(candidates) => candidates,
                            Err(e) => {
                                eprintln!(
                                        "Warning: Workshop detail lookup for \"{}\" failed ({}), will retry next run",
                                        variation, e
                                    );
                                continue;
                            }
                        }
                    } else {
                        scraped
                    }
                }
            };

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
            let cached: Vec<ScoredCandidate> = merged.values().cloned().collect();
            cache.insert(variation.clone(), cached);
            save_identity_cache(cache);
            searched.insert(variation.clone());

            // Rate-limit only real Workshop page hits
            if !any_from_cache {
                std::thread::sleep(std::time::Duration::from_millis(1100));
            }
        }

        let candidates: Vec<ScoredCandidate> = merged.into_values().collect();
        // Cache under the primary term too
        if !candidates.is_empty() {
            let cached: Vec<ScoredCandidate> = candidates.clone();
            cache.insert(term.clone(), cached);
            save_identity_cache(cache);
        }
        (candidates, any_from_cache)
    };

    if candidates.is_empty() {
        counters.no_candidate += 1;
        for pbo in &group.pbos {
            println!(
                "  {} {} {}: no candidates for \"{}\"",
                paint(use_colour, C_DIM, progress),
                paint(use_colour, C_RED, "[none]"),
                pbo,
                term
            );
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
        let title_match = title_contains_term(&top_title_lower, &term_lower);

        // Character-level token overlap for the top candidate
        let top_word_score = word_overlap_score(&term_lower, &top_title_lower);

        // Also check word-level token relevance
        let term_tokens = split_camel_or_underscore(&term_lower);
        let top_token_hits = term_tokens
            .iter()
            .filter(|t| t.len() >= 3 && title_contains_term(&top_title_lower, t))
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
                        score,
                        human_count(c.subscriptions),
                        human_count(c.views)
                    )
                } else {
                    format!(" [score={:.0}]", score)
                };
                format!("{} ({}){}", sanitize_for_terminal(&c.title), c.id, stats)
            })
            .collect::<Vec<_>>();

        // Group header: status symbol, progress, PBOs, search meta.
        let header_meta = if meta_str.is_empty() {
            format!(", {} candidate(s)", candidates.len())
        } else {
            format!(", {} candidate(s){}", candidates.len(), meta_str)
        };

        if weak_match {
            counters.weak += 1;
            println!(
                "  {} {} (search \"{}\"{})",
                paint(use_colour, C_YELLOW, &format!("? {}", progress)),
                group.pbos.join(", "),
                term,
                header_meta
            );
            if let Some(best) = top.first() {
                println!("      best: {} — no confident match", best);
            }
        } else {
            counters.confident += 1;
            println!(
                "  {} {} (search \"{}\"{})",
                paint(use_colour, C_GREEN, &format!("\u{2713} {}", progress)),
                group.pbos.join(", "),
                term,
                header_meta
            );
            for candidate in top {
                println!("      \u{2192} {}", candidate);
            }
        }

        // Changelog cross-reference: if the top candidate has a
        // changelog, check if it names mod families from our group.
        // This catches cases where a pack lists its source mods.
        if !from_cache && !scored.is_empty() {
            if let Some((_score, top_candidate)) = scored.first() {
                let notes = scrape_changelog(&top_candidate.id, api_client);
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
                                sanitize_for_terminal(truncate_chars(note, 100)),
                                sanitize_for_terminal(pbo_name)
                            );
                            break;
                        }
                    }
                }
            }
        }
    }
}
