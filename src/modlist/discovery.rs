use std::collections::{HashMap, HashSet};

use super::deps::{fetch_workshop_dependencies, is_non_mod_app_id};
use super::ModEntry;

/// Direct Workshop dependencies per fetched id: id -> [(dep_id, name)].
pub type DepMap = HashMap<String, Vec<(String, String)>>;

/// Keep only dependencies that are not app ids, known roots, ignored ids,
/// or duplicates. First-seen order is preserved.
pub(crate) fn filter_allowed(
    deps: Vec<(String, String)>,
    known: &HashSet<String>,
    ignored: &HashSet<String>,
) -> Vec<(String, String)> {
    let mut seen: HashSet<String> = HashSet::new();
    deps.into_iter()
        .filter(|(id, _)| {
            !is_non_mod_app_id(id)
                && !known.contains(id)
                && !ignored.contains(id)
                && seen.insert(id.clone())
        })
        .collect()
}

/// Depth-first closure of each root over the direct map. A root is never
/// part of its own closure. Cycles terminate through the visited set.
pub fn transitive_closure(roots: &[String], direct: &DepMap) -> DepMap {
    // A dependency can appear under several parents; keep its first name.
    let mut names: HashMap<String, String> = HashMap::new();
    for deps in direct.values() {
        for (id, name) in deps {
            names.entry(id.clone()).or_insert_with(|| name.clone());
        }
    }

    let mut out = DepMap::new();
    for root in roots {
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(root.clone());
        let mut order: Vec<String> = Vec::new();
        let mut stack: Vec<String> = direct
            .get(root)
            .into_iter()
            .flatten()
            .map(|(id, _)| id.clone())
            .collect();

        while let Some(id) = stack.pop() {
            if !visited.insert(id.clone()) {
                continue;
            }
            order.push(id.clone());
            if let Some(children) = direct.get(&id) {
                for (child_id, _) in children {
                    if !visited.contains(child_id) {
                        stack.push(child_id.clone());
                    }
                }
            }
        }

        let closure = order
            .into_iter()
            .map(|id| {
                let name = names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| format!("Mod {id}"));
                (id, name)
            })
            .collect();
        out.insert(root.clone(), closure);
    }
    out
}

/// Fetch the Workshop page of every enabled root, then every allowed
/// dependency reached from it. Returns the deduplicated union of all
/// discovered dependencies and the closure per root.
///
/// One request per second keeps the fetches within Steam's tolerance.
pub fn resolve_all_required(
    roots: &[ModEntry],
    known: &HashSet<String>,
    ignored: &HashSet<String>,
) -> (Vec<(String, String)>, DepMap) {
    let mut direct = DepMap::new();
    let mut fetched: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = roots.iter().map(|m| m.id.clone()).collect();

    while let Some(id) = queue.pop() {
        if !fetched.insert(id.clone()) {
            continue;
        }
        let deps = fetch_workshop_dependencies(&id);
        let allowed = filter_allowed(deps, known, ignored);
        for (dep_id, _) in &allowed {
            if !fetched.contains(dep_id) {
                queue.push(dep_id.clone());
            }
        }
        direct.insert(id, allowed);
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    let root_ids: Vec<String> = roots.iter().map(|m| m.id.clone()).collect();
    let by_root = transitive_closure(&root_ids, &direct);

    let mut all: Vec<(String, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for deps in by_root.values() {
        for (id, name) in deps {
            if seen.insert(id.clone()) {
                all.push((id.clone(), name.clone()));
            }
        }
    }
    (all, by_root)
}

/// Print a mod's missing dependency tree with tree-style indentation.
/// Only shows deps that were themselves fetched (i.e. also missing).
pub fn print_dep_tree(id: &str, deps_by_mod: &DepMap, prefix: &str, visited: &mut HashSet<String>) {
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

    fn dep(id: &str, name: &str) -> (String, String) {
        (id.to_string(), name.to_string())
    }

    #[test]
    fn transitive_closure_follows_chain() {
        // A -> B -> C
        let mut direct = DepMap::new();
        direct.insert("A".to_string(), vec![dep("B", "Bee")]);
        direct.insert("B".to_string(), vec![dep("C", "Cee")]);
        let closure = transitive_closure(&["A".to_string()], &direct);
        let under_a = &closure["A"];
        assert!(under_a.iter().any(|(id, _)| id == "B"));
        assert!(under_a.iter().any(|(id, _)| id == "C"));
        assert!(under_a.iter().all(|(id, _)| id != "A"));
    }

    #[test]
    fn transitive_closure_handles_cycle() {
        // A <-> B must terminate and list each other once.
        let mut direct = DepMap::new();
        direct.insert("A".to_string(), vec![dep("B", "Bee")]);
        direct.insert("B".to_string(), vec![dep("A", "Ay")]);
        let closure = transitive_closure(&["A".to_string()], &direct);
        assert_eq!(closure["A"].len(), 1);
        assert_eq!(closure["A"][0].0, "B");
    }

    #[test]
    fn filter_allowed_drops_app_known_ignored_and_dupes() {
        let deps = vec![
            dep("107410", "Arma 3"),
            dep("228800", "DayZ"),
            dep("450814997", "CBA_A3"),
            dep("450814997", "CBA_A3 duplicate"),
            dep("9999999999", "Known root"),
            dep("8888888888", "Ignored"),
            dep("7777777777", "Kept"),
        ];
        let known: HashSet<String> = ["9999999999".to_string()].into_iter().collect();
        let ignored: HashSet<String> = ["8888888888".to_string()].into_iter().collect();
        let filtered = filter_allowed(deps, &known, &ignored);
        assert_eq!(
            filtered,
            vec![dep("450814997", "CBA_A3"), dep("7777777777", "Kept")]
        );
    }
}
