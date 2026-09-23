//! The online phase: visibility check, grouped Workshop search, summary.

use std::collections::HashSet;
use std::path::Path;

use super::bikey::scan_bikey_names;
use super::cache::load_identity_cache;
use super::group::group_pbos_by_term;
use super::online_group::{search_group, GroupCounters};
use super::visibility::check_workshop_visibility;
use crate::origin::ResolvedOrigin;
use crate::util::{paint, C_BOLD, C_DIM};
use crate::workshop_api::build_client;

/// Check each resolved origin against the Workshop, then search the
/// Workshop for every PBO whose origin is unknown or an aggregate pack.
pub(crate) fn search_online(
    results: &[(String, Option<ResolvedOrigin>)],
    addons_dir: &Path,
    use_colour: bool,
) {
    check_workshop_visibility(results.to_vec());

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
    let total_groups = groups.len();
    let mut cache = load_identity_cache();
    let api_client = build_client();
    let mut searched = HashSet::new();
    let mut counters = GroupCounters::default();

    for (group_idx, group) in groups.iter().enumerate() {
        let progress = format!("[{}/{}]", group_idx + 1, total_groups);
        search_group(
            group,
            &progress,
            use_colour,
            &api_client,
            &mut cache,
            &mut searched,
            &mut counters,
        );
    }

    // Final summary
    println!();
    println!("{}", paint(use_colour, C_BOLD, "Summary:"));
    println!(
        "  {} confident match(es) — the top candidate's title matches the search term",
        counters.confident
    );
    println!(
        "  {} weak match(es) — no confident match, best candidate shown for reference",
        counters.weak
    );
    println!("  {} group(s) with no candidates", counters.no_candidate);
    if counters.cached > 0 {
        println!(
            "  {} group(s) served from the local cache ({} total)",
            counters.cached,
            paint(
                use_colour,
                C_DIM,
                "run 'uksfta investigate --online' to refresh"
            )
        );
    }
}
