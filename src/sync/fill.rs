//! Satisfy discovered dependencies from an Arma 3 launcher modlist. The
//! caller parses the modlist and expands its collections; every function
//! here is pure so the classification needs no network access.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::modlist::{parse_modlist_html, DepMap, ModEntry};

/// A required dependency the launcher modlist already provides.
pub(crate) struct SatisfiedDep {
    pub id: String,
    pub name: String,
    pub required_by: String,
    pub required_by_name: String,
}

/// Workshop ids listed directly in a launcher modlist document.
pub(crate) fn provided_ids(content: &str) -> HashSet<String> {
    let (found, _local) = parse_modlist_html(content);
    found.into_iter().map(|(id, _)| id).collect()
}

/// Merge expanded collection members into the provided set.
pub(crate) fn merge_members(provided: &mut HashSet<String>, members: &[String]) {
    provided.extend(members.iter().cloned());
}

/// Classify every required dependency the provided set satisfies, in id
/// order and without duplicates. A known root is never a dependency, so it
/// is never satisfied here.
pub(crate) fn satisfied_dependencies(
    mods: &[ModEntry],
    discovered: &DepMap,
    provided: &HashSet<String>,
) -> Vec<SatisfiedDep> {
    let known: HashSet<String> = mods.iter().map(|m| m.id.clone()).collect();
    let mut names: HashMap<String, String> = HashMap::new();
    for deps in discovered.values() {
        for (id, name) in deps {
            names.entry(id.clone()).or_insert_with(|| name.clone());
        }
    }

    let mut classify = Classify {
        provided,
        known: &known,
        names: &names,
        seen: HashSet::new(),
        out: Vec::new(),
    };

    for entry in mods {
        for dep_id in &entry.dependencies {
            classify.push(dep_id, &entry.id, &entry.name);
        }
    }
    for (root_id, deps) in discovered {
        let root_name = mods
            .iter()
            .find(|m| &m.id == root_id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("Mod {root_id}"));
        for (dep_id, _) in deps {
            classify.push(dep_id, root_id, &root_name);
        }
    }

    let mut out = classify.out;
    out.sort_by(|a, b| {
        a.id.cmp(&b.id)
            .then_with(|| a.required_by.cmp(&b.required_by))
    });
    out
}

/// The distinct ids to treat as ignored after fill, sorted by id.
pub(crate) fn ignore_ids(satisfied: &[SatisfiedDep]) -> Vec<String> {
    ignore_entries(satisfied)
        .into_iter()
        .map(|(id, _)| id)
        .collect()
}

/// The distinct (id, name) pairs to persist as ignore blocks, sorted by id.
pub(crate) fn ignore_entries(satisfied: &[SatisfiedDep]) -> Vec<(String, String)> {
    let mut by_id: BTreeMap<String, String> = BTreeMap::new();
    for s in satisfied {
        by_id.entry(s.id.clone()).or_insert_with(|| s.name.clone());
    }
    by_id.into_iter().collect()
}

struct Classify<'a> {
    provided: &'a HashSet<String>,
    known: &'a HashSet<String>,
    names: &'a HashMap<String, String>,
    seen: HashSet<(String, String)>,
    out: Vec<SatisfiedDep>,
}

impl Classify<'_> {
    fn push(&mut self, dep_id: &str, root_id: &str, root_name: &str) {
        if self.known.contains(dep_id) || !self.provided.contains(dep_id) {
            return;
        }
        if !self.seen.insert((dep_id.to_string(), root_id.to_string())) {
            return;
        }
        let name = self
            .names
            .get(dep_id)
            .cloned()
            .unwrap_or_else(|| format!("Mod {dep_id}"));
        self.out.push(SatisfiedDep {
            id: dep_id.to_string(),
            name,
            required_by: root_id.to_string(),
            required_by_name: root_name.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::lock::LockFile;
    use crate::sync::expand::expand_dependencies;
    use crate::sync::plan::SyncPlan;
    use crate::sync::unpullable_dependencies;

    const MODLIST: &str = r#"<html><body><table>
<tr data-type="ModContainer">
<td data-type="DisplayName">CBA_A3</td>
<td><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=450814997" data-type="Link">URL</a></td>
</tr>
<tr data-type="ModContainer">
<td data-type="DisplayName">Mod 3299910335</td>
<td><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=3299910335" data-type="Link">URL</a></td>
</tr>
</table></body></html>"#;

    fn root(deps: Vec<&str>) -> ModEntry {
        ModEntry {
            id: "3299910335".to_string(),
            name: "Mod 3299910335".to_string(),
            tags: Vec::new(),
            role: "mod".to_string(),
            enabled: true,
            dependencies: deps.into_iter().map(String::from).collect(),
        }
    }

    fn discovered_with_cba() -> DepMap {
        let mut map = DepMap::new();
        map.insert(
            "3299910335".to_string(),
            vec![("450814997".to_string(), "CBA_A3".to_string())],
        );
        map
    }

    fn empty_cache(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("uksfta-fill-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        let cache = root.join("107410");
        std::fs::create_dir_all(&cache).unwrap();
        (root, cache)
    }

    #[test]
    fn provided_ids_reads_modlist() {
        let ids = provided_ids(MODLIST);
        assert!(ids.contains("450814997"));
        assert!(ids.contains("3299910335"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn merge_members_adds_collection_members() {
        let mut provided = provided_ids(MODLIST);
        merge_members(
            &mut provided,
            &["1111111111".to_string(), "2222222222".to_string()],
        );
        assert!(provided.contains("1111111111"));
        assert!(provided.contains("2222222222"));
        assert!(provided.contains("450814997"));
        assert_eq!(provided.len(), 4);
    }

    #[test]
    fn provided_dependency_is_satisfied() {
        let mods = vec![root(Vec::new())];
        let provided: HashSet<String> = ["450814997".to_string()].into_iter().collect();
        let satisfied = satisfied_dependencies(&mods, &discovered_with_cba(), &provided);
        assert_eq!(satisfied.len(), 1);
        assert_eq!(satisfied[0].id, "450814997");
        assert_eq!(satisfied[0].name, "CBA_A3");
        assert_eq!(satisfied[0].required_by, "3299910335");
    }

    #[test]
    fn unprovided_dependency_is_not_satisfied() {
        let mods = vec![root(Vec::new())];
        let satisfied = satisfied_dependencies(&mods, &discovered_with_cba(), &HashSet::new());
        assert!(satisfied.is_empty());
    }

    #[test]
    fn known_root_is_never_satisfied() {
        let mods = vec![root(vec!["3299910335"])];
        let provided: HashSet<String> = ["3299910335".to_string()].into_iter().collect();
        let satisfied = satisfied_dependencies(&mods, &DepMap::new(), &provided);
        assert!(satisfied.is_empty());
    }

    #[test]
    fn ignore_ids_dedupe_by_id() {
        let satisfied = vec![
            SatisfiedDep {
                id: "1".to_string(),
                name: "A".to_string(),
                required_by: "9".to_string(),
                required_by_name: "R".to_string(),
            },
            SatisfiedDep {
                id: "1".to_string(),
                name: "A".to_string(),
                required_by: "8".to_string(),
                required_by_name: "S".to_string(),
            },
        ];
        assert_eq!(ignore_ids(&satisfied), vec!["1".to_string()]);
    }

    #[test]
    fn provided_dependency_is_not_planned_for_repack() {
        let (root_dir, cache) = empty_cache("no-repack");
        let mods = vec![root(Vec::new())];
        let provided: HashSet<String> = ["450814997".to_string()].into_iter().collect();
        let discovered = discovered_with_cba();
        let satisfied = satisfied_dependencies(&mods, &discovered, &provided);
        let ignored = ignore_ids(&satisfied);
        let lock = LockFile {
            version: 1,
            mods: HashMap::new(),
        };

        let expansion = expand_dependencies(
            &mods,
            &ignored,
            std::slice::from_ref(&cache),
            &lock,
            &HashMap::new(),
            &discovered,
        );

        assert!(expansion.planned.is_empty());
        assert!(!expansion.required.contains("450814997"));

        std::fs::remove_dir_all(&root_dir).ok();
    }

    #[test]
    fn unprovided_cached_dependency_is_repacked() {
        let (root_dir, cache) = empty_cache("repack");
        let dep_dir = cache.join("450814997");
        std::fs::create_dir_all(&dep_dir).unwrap();
        std::fs::write(dep_dir.join("x.pbo"), b"pbo").unwrap();

        let mods = vec![root(Vec::new())];
        let discovered = discovered_with_cba();
        let satisfied = satisfied_dependencies(&mods, &discovered, &HashSet::new());
        assert!(satisfied.is_empty());
        let lock = LockFile {
            version: 1,
            mods: HashMap::new(),
        };

        let expansion = expand_dependencies(
            &mods,
            &[],
            std::slice::from_ref(&cache),
            &lock,
            &HashMap::new(),
            &discovered,
        );

        assert!(expansion.planned.iter().any(|p| p.id == "450814997"));

        std::fs::remove_dir_all(&root_dir).ok();
    }

    #[test]
    fn unprovided_uncached_dependency_hard_fails() {
        let (root_dir, cache) = empty_cache("hard-fail");
        let mods = vec![root(Vec::new())];
        let discovered = discovered_with_cba();
        let lock = LockFile {
            version: 1,
            mods: HashMap::new(),
        };

        let expansion = expand_dependencies(
            &mods,
            &[],
            std::slice::from_ref(&cache),
            &lock,
            &HashMap::new(),
            &discovered,
        );

        assert_eq!(expansion.missing.len(), 1);
        let plan = SyncPlan {
            planned: Vec::new(),
            missing_from_cache: expansion.missing.clone(),
            removed: Vec::new(),
            added: 0,
            updated: 0,
            unchanged: 0,
            discovered_deps: discovered.clone(),
            required_deps: expansion.required.clone(),
        };
        assert_eq!(unpullable_dependencies(&plan), vec!["450814997"]);

        std::fs::remove_dir_all(&root_dir).ok();
    }
}
