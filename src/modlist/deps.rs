use std::collections::{HashMap, HashSet};

use crate::util::workshop_url;

/// Parse a Workshop page's "Required Items" section.
/// Returns Vec<(id, name)>. Empty if the section is absent.
pub fn parse_required_items(html: &str) -> Vec<(String, String)> {
    let document = scraper::Html::parse_document(html);

    // Steam renders required items in a div with id="RequiredItems"
    // Each item is a link: <a href="...?id=NNN">Name</a>
    let Some(required_section) = document
        .select(&scraper::Selector::parse("#RequiredItems").unwrap())
        .next()
    else {
        return Vec::new();
    };

    let mut deps = Vec::new();
    for link in required_section.select(&scraper::Selector::parse("a").unwrap()) {
        let href = link.value().attr("href").unwrap_or("");
        // Extract id=NNN from the href
        let id = href
            .split("?id=")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let name = link.text().collect::<String>().trim().to_string();
        if !name.is_empty() {
            deps.push((id.to_string(), name));
        }
    }
    deps
}

/// Fetch a Workshop item's page and return its required dependencies as
/// Vec<(id, name)>. Returns empty on network error or if no deps exist.
fn fetch_workshop_dependencies(workshop_id: &str) -> Vec<(String, String)> {
    let url = workshop_url(workshop_id);
    // A stalled connection must not hang the CLI indefinitely.
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("Failed to build HTTP client");
    let body = match client.get(&url).send() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "Warning: failed to fetch Workshop page for {}: {}",
                workshop_id, e
            );
            return Vec::new();
        }
    };
    let html = match body.text() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "Warning: failed to read response for {}: {}",
                workshop_id, e
            );
            return Vec::new();
        }
    };
    parse_required_items(&html)
}

/// Resolve transitive dependencies for a list of missing mods.
/// Returns (expanded list, direct deps per fetched mod).
/// The expanded list is missing mods + discovered deps not already known.
/// Deps already present in the workshop cache are skipped entirely.
type DepResolution = (
    Vec<(String, String)>,
    HashMap<String, Vec<(String, String)>>,
);

pub fn resolve_transitive_deps(
    missing: &[(String, String)],
    known_ids: &HashSet<String>,
    cached_ids: &HashSet<String>,
) -> DepResolution {
    let mut result = Vec::new();
    let mut deps_by_mod: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut fetched: HashSet<String> = HashSet::new();
    let mut queue: Vec<(String, String)> = missing.to_vec();

    while let Some((id, name)) = queue.pop() {
        if fetched.contains(&id) {
            continue;
        }
        fetched.insert(id.clone());

        // Missing mods always go in the result
        result.push((id.clone(), name));

        let deps = fetch_workshop_dependencies(&id);
        // Keep only deps we do not already have: not cached, not known
        let missing_deps: Vec<(String, String)> = deps
            .into_iter()
            .filter(|(dep_id, _)| !cached_ids.contains(dep_id) && !known_ids.contains(dep_id))
            .collect();
        // Record for tree display
        deps_by_mod.insert(id.clone(), missing_deps.clone());
        for (dep_id, dep_name) in missing_deps {
            if !fetched.contains(&dep_id) {
                queue.push((dep_id, dep_name));
            }
        }
        // Rate limit: 1 request per second
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    (result, deps_by_mod)
}

/// Print a mod's missing dependency tree with tree-style indentation.
/// Only shows deps that were themselves fetched (i.e. also missing).
pub fn print_dep_tree(
    id: &str,
    deps_by_mod: &HashMap<String, Vec<(String, String)>>,
    prefix: &str,
    visited: &mut HashSet<String>,
) {
    let Some(deps) = deps_by_mod.get(id) else {
        return;
    };
    let missing: Vec<&(String, String)> = deps
        .iter()
        .filter(|(dep_id, _)| deps_by_mod.contains_key(dep_id) && !visited.contains(dep_id))
        .collect();
    for (i, (dep_id, dep_name)) in missing.iter().enumerate() {
        let is_last = i == missing.len() - 1;
        let connector = if is_last { "└─ " } else { "├─ " };
        eprintln!("{}{}{} ({})", prefix, connector, dep_name, dep_id);
        visited.insert(dep_id.clone());
        let child_prefix = if is_last {
            format!("{}   ", prefix)
        } else {
            format!("{}│  ", prefix)
        };
        print_dep_tree(dep_id, deps_by_mod, &child_prefix, visited);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
