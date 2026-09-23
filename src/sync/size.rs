use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::error::UksftaError;
use crate::modlist::parse_modlist_html;
use crate::steam::{fetch_workshop_sizes, load_workshop_items, WorkshopItem};
use crate::util::{human_size, paint, stdout_is_tty, C_BOLD, C_DIM, C_GREEN, C_RED, C_YELLOW};

/// Print each Steam mod in an Arma 3 launcher modlist HTML file with its
/// download size from the local Workshop cache, then the total. With
/// `online`, sizes for mods not installed locally come from the keyless
/// GetPublishedFileDetails API.
pub fn modlist_size(modlist_file: &Path, online: bool) -> Result<(), UksftaError> {
    let content = fs::read_to_string(modlist_file).map_err(|e| {
        UksftaError::Input(format!("cannot read {}: {}", modlist_file.display(), e))
    })?;

    let (steam_mods, local_mods) = parse_modlist_html(&content);
    if steam_mods.is_empty() && local_mods.is_empty() {
        return Err(UksftaError::Input(format!(
            "No mods found in {}. Is it an Arma 3 launcher modlist?",
            modlist_file.display()
        )));
    }

    let mut items = load_workshop_items();
    if online {
        let missing = missing_size_ids(&steam_mods, &items);
        if !missing.is_empty() {
            println!(
                "Fetching {} mod size(s) from the Steam Workshop API...",
                missing.len()
            );
            for (id, size) in fetch_workshop_sizes(&missing) {
                // Only the size is used from here on; the timestamp is unused.
                items.insert(
                    id,
                    WorkshopItem {
                        size,
                        time_updated: 0,
                    },
                );
            }
        }
    }
    let sizes = lookup_sizes(&steam_mods, &items);
    let use_colour = stdout_is_tty();
    let total: u64 = sizes.iter().filter_map(|(_, _, size)| *size).sum();

    println!("Modlist size:");
    println!("  {} mod(s) in preset", sizes.len());
    if use_colour {
        println!(
            "  {}",
            paint(
                use_colour,
                C_DIM,
                "colour: green <5%, yellow <20%, red >=20% of total"
            )
        );
    }

    let mut unknown = 0;
    for (id, name, size) in &sizes {
        match size {
            Some(bytes) => {
                let share = if total > 0 {
                    *bytes as f64 / total as f64
                } else {
                    0.0
                };
                let cell = format!("{:>10}", human_size(*bytes));
                println!(
                    "  {}  {} ({})",
                    paint(use_colour, size_share_colour(share), &cell),
                    name,
                    id
                );
            }
            None => {
                unknown += 1;
                let cell = format!("{:>10}", "unknown");
                println!("  {}  {} ({})", paint(use_colour, C_DIM, &cell), name, id);
            }
        }
    }

    println!();
    println!("{}", paint(use_colour, C_BOLD, "Summary:"));
    println!(
        "  Total: {} ({} of {} mods)",
        paint(use_colour, C_BOLD, &human_size(total)),
        sizes.len() - unknown,
        sizes.len()
    );
    if unknown > 0 {
        let hint = if online {
            String::new()
        } else {
            paint(
                use_colour,
                C_DIM,
                " Run with --online to fetch the missing sizes.",
            )
        };
        println!("  {} mod(s) not in the Workshop cache.{}", unknown, hint);
    }
    if !local_mods.is_empty() {
        println!(
            "  {} local mod(s) with no Workshop size: {}",
            local_mods.len(),
            local_mods.join(", ")
        );
    }
    Ok(())
}

/// Colour for a mod's share of the total size: green under 5%, yellow
/// under 20%, red at or above 20%.
pub fn size_share_colour(share: f64) -> &'static str {
    if share < 0.05 {
        C_GREEN
    } else if share < 0.20 {
        C_YELLOW
    } else {
        C_RED
    }
}

/// Pair each mod with its Workshop cache size. The size is None when the
/// mod is not installed locally, so the total counts only known sizes.
pub fn lookup_sizes(
    mods: &[(String, String)],
    items: &HashMap<String, WorkshopItem>,
) -> Vec<(String, String, Option<u64>)> {
    mods.iter()
        .map(|(id, name)| (id.clone(), name.clone(), items.get(id).map(|i| i.size)))
        .collect()
}

/// Deduplicated ids from `mods` whose size is not known locally. A size of
/// zero counts as unknown, since the ACF omits it for uninstalled items.
pub fn missing_size_ids(
    mods: &[(String, String)],
    items: &HashMap<String, WorkshopItem>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for (id, _) in mods {
        let known = items.get(id).map(|i| i.size > 0).unwrap_or(false);
        if !known && seen.insert(id.clone()) {
            ids.push(id.clone());
        }
    }
    ids
}
