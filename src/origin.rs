use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::pbo::pbo_prefix;
use crate::prefix::prefix_root;
use crate::util::{file_sha256, find_pbos};

/// A Workshop mod folder plus precomputed statistics used to arbitrate
/// origin: how many PBOs it holds and how many distinct prefix roots
/// (a standalone mod has few roots; an aggregate pack spans many).
pub struct ModDir {
    pub id: String,
    pub path: PathBuf,
    pub pbos: Vec<PathBuf>,
    pub root_count: usize,
}

pub fn build_mod_dirs(caches: &[PathBuf]) -> Vec<ModDir> {
    let mut dirs = Vec::new();
    for cache in caches {
        if let Ok(entries) = fs::read_dir(cache) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(id) = path.file_name().and_then(|n| n.to_str()) {
                        let pbos = find_pbos(&path);
                        let root_count = {
                            let mut roots = std::collections::HashSet::new();
                            for pbo in &pbos {
                                if let Some(prefix) = pbo_prefix(pbo) {
                                    roots.insert(prefix_root(&prefix).to_lowercase());
                                }
                            }
                            roots.len()
                        };
                        dirs.push(ModDir {
                            id: id.to_string(),
                            path,
                            pbos,
                            root_count,
                        });
                    }
                }
            }
        }
    }
    dirs
}

/// The Workshop ID of the current directory, if the current directory is
/// itself a mod folder inside a Workshop cache (e.g. investigating a pack's
/// own contents). Such a folder must not be a candidate for its own PBOs.
pub fn self_workshop_id(caches: &[PathBuf]) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let cwd = fs::canonicalize(&cwd).ok()?;
    for cache in caches {
        let Ok(cache) = fs::canonicalize(cache) else {
            continue;
        };
        if cwd.starts_with(&cache) {
            // The mod folder is cwd relative to cache
            if let Ok(rel) = cwd.strip_prefix(&cache) {
                let components: Vec<_> = rel.components().collect();
                if components.len() == 1 {
                    return components[0].as_os_str().to_str().map(|s| s.to_string());
                }
            }
        }
    }
    None
}

/// The resolved origin of a PBO: the Workshop ID plus the prefix that
/// identified it, and whether the match was itself an aggregate pack.
#[derive(Debug, PartialEq, Clone)]
pub struct ResolvedOrigin {
    pub id: String,
    pub prefix: Option<String>,
    pub is_pack: bool,
}

/// The local identity cache type: search term -> candidate entries,
/// persisted to .uksfta/identities.json. The entry type is generic so
/// each consumer stores the data it needs (ScoredCandidate in the
/// investigate command, plain id/title pairs in tests).
pub type IdentityCache<T> = HashMap<String, Vec<T>>;

/// Build a single-pass index of every PBO name to the mod folders that
/// contain it. This is the expensive cache walk; resolving then becomes
/// in-memory lookups instead of re-scanning folders per PBO.
pub fn build_pbo_index(mod_dirs: &[ModDir]) -> HashMap<String, Vec<&ModDir>> {
    let mut index: HashMap<String, Vec<&ModDir>> = HashMap::new();
    for dir in mod_dirs {
        for pbo in &dir.pbos {
            if let Some(name) = pbo.file_name().and_then(|n| n.to_str()) {
                index.entry(name.to_string()).or_default().push(dir);
            }
        }
    }
    index
}

/// Match a PBO against the Workshop cache and arbitrate its origin.
/// Primary signal: the PBO header prefix, which identifies the original
/// addon and survives re-packing. Byte-hash is a fallback for PBOs
/// without a readable prefix.
///
/// Among folders carrying the same prefix, prefer the one with the fewest
/// distinct prefix roots: a standalone mod (e.g. ACE) has one root, while
/// an aggregate pack spans many. This steers the result to the original
/// owner mod over any pack that bundled a copy.
///
/// `candidates` is the prebuilt per-name folder list from the index.
pub fn resolve_pbo_from_index(
    pbo_path: &Path,
    candidates: &[&ModDir],
    target_prefix: Option<&str>,
) -> Option<ResolvedOrigin> {
    let pbo_name = pbo_path.file_name()?.to_string_lossy().to_string();

    // Score each candidate. Prefix match is the primary signal and is
    // cheap (512-byte header read). Byte-hash is a fallback used only
    // when the prefix path fails, since hashing reads the whole PBO.
    // The target hash is computed once: a single unreadable target
    // leaves every byte-hash candidate unscored but does not abort.
    let target_hash = match target_prefix {
        Some(_) => None,
        None => Some(file_sha256(pbo_path)?),
    };
    let mut scored: Vec<(&ModDir, u8)> = Vec::new();
    for dir in candidates {
        let Some(cached_path) = dir.pbos.iter().find(|pbo| {
            pbo.file_name()
                .map(|n| n == pbo_name.as_str())
                .unwrap_or(false)
        }) else {
            continue;
        };
        let mut score = 0u8;
        if let Some(target) = target_prefix {
            if pbo_prefix(cached_path).as_deref() == Some(target) {
                score += 2; // same canonical addon prefix
            }
        } else {
            // No prefix available: fall back to byte identity.
            if file_sha256(cached_path) == target_hash {
                score += 1;
            }
        }
        if score > 0 {
            scored.push((dir, score));
        }
    }

    if scored.is_empty() {
        return None;
    }
    let best_score = scored.iter().map(|(_, s)| *s).max().unwrap_or(0);

    // Among the highest-scoring folders, pick the one with the fewest
    // distinct prefix roots (standalone mod over aggregate pack).
    let best = scored
        .into_iter()
        .filter(|(_, s)| *s == best_score)
        .min_by_key(|(dir, _)| dir.root_count)?
        .0;

    // A folder is an aggregate pack when it spans a very large number of
    // distinct prefix roots. Large standalone mods (e.g. FZA AH-64 with 28
    // roots) stay below the threshold; the known aggregate packs here span
    // 61 and 159 roots. This flag is a warning to verify, not a guarantee.
    let is_pack = best.root_count > 50;

    Some(ResolvedOrigin {
        id: best.id.clone(),
        prefix: target_prefix.map(|s| s.to_string()),
        is_pack,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a fake PBO with a header carrying the given prefix.
    fn write_pbo(path: &Path, prefix: &str, payload: &[u8]) {
        // Mimic the real PBO header: b"\x00sreV" then "prefix\0{prefix}\0".
        let mut data = b"\x00sreV\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        data.extend_from_slice(b"prefix\x00");
        data.extend_from_slice(prefix.as_bytes());
        data.push(0);
        data.extend_from_slice(payload);
        fs::write(path, data).unwrap();
    }

    #[test]
    fn resolve_pbo_origin_prefers_standalone_over_pack() {
        let dir = std::env::temp_dir().join("uksfta-origin-test");
        fs::create_dir_all(&dir).unwrap();

        // mod 111 (standalone, 1 PBO) and mod 222 (pack, 50 PBOs) both
        // contain an identical copy of common.pbo
        let standalone = dir.join("111").join("addons");
        let pack = dir.join("222").join("addons");
        fs::create_dir_all(&standalone).unwrap();
        fs::create_dir_all(&pack).unwrap();
        let content = b"identical pbo bytes";
        fs::write(standalone.join("common.pbo"), content).unwrap();
        fs::write(pack.join("common.pbo"), content).unwrap();
        for i in 0..50 {
            fs::write(pack.join(format!("pack_{}.pbo", i)), format!("pack{}", i)).unwrap();
        }

        let mod_dirs = vec![
            ModDir {
                id: "222".to_string(),
                path: dir.join("222"),
                pbos: find_pbos(&dir.join("222")),
                root_count: 50,
            },
            ModDir {
                id: "111".to_string(),
                path: dir.join("111"),
                pbos: find_pbos(&dir.join("111")),
                root_count: 1,
            },
        ];
        let index = build_pbo_index(&mod_dirs);

        let target = dir.join("common.pbo");
        fs::write(&target, content).unwrap();

        // Arbitration must pick the standalone mod (111) with fewer roots
        let candidates = index.get("common.pbo").map(|v| v.as_slice()).unwrap_or(&[]);
        assert_eq!(
            resolve_pbo_from_index(&target, candidates, pbo_prefix(&target).as_deref())
                .unwrap()
                .id,
            "111".to_string()
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_pbo_origin_no_match_returns_none() {
        let dir = std::env::temp_dir().join("uksfta-origin-none");
        fs::create_dir_all(&dir).unwrap();
        let mod_dir = dir.join("111").join("addons");
        fs::create_dir_all(&mod_dir).unwrap();
        fs::write(mod_dir.join("other.pbo"), b"different").unwrap();

        let target = dir.join("target.pbo");
        fs::write(&target, b"no match anywhere").unwrap();

        let mod_dirs = vec![ModDir {
            id: "111".to_string(),
            path: dir.join("111"),
            pbos: find_pbos(&dir.join("111")),
            root_count: 1,
        }];
        let index = build_pbo_index(&mod_dirs);
        let candidates = index.get("target.pbo").map(|v| v.as_slice()).unwrap_or(&[]);
        assert_eq!(
            resolve_pbo_from_index(&target, candidates, pbo_prefix(&target).as_deref()),
            None
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn resolve_pbo_origin_matches_by_prefix_over_differing_bytes() {
        // Two folders hold same-named PBOs with the same prefix but
        // different bytes (a pack repacked the mod's copy). The origin
        // must be the folder with fewer roots (the standalone mod).
        let dir = std::env::temp_dir().join("uksfta-prefix-origin");
        fs::create_dir_all(&dir).unwrap();
        let standalone = dir.join("111").join("addons");
        let pack = dir.join("222").join("addons");
        fs::create_dir_all(&standalone).unwrap();
        fs::create_dir_all(&pack).unwrap();
        write_pbo(
            &standalone.join("common.pbo"),
            "z\\ace\\addons\\common",
            b"version-a",
        );
        write_pbo(
            &pack.join("common.pbo"),
            "z\\ace\\addons\\common",
            b"version-b",
        );

        let mod_dirs = vec![
            ModDir {
                id: "222".to_string(),
                path: dir.join("222"),
                pbos: find_pbos(&dir.join("222")),
                root_count: 50,
            },
            ModDir {
                id: "111".to_string(),
                path: dir.join("111"),
                pbos: find_pbos(&dir.join("111")),
                root_count: 1,
            },
        ];
        let index = build_pbo_index(&mod_dirs);

        let target = dir.join("common.pbo");
        write_pbo(&target, "z\\ace\\addons\\common", b"version-a");

        let candidates = index.get("common.pbo").map(|v| v.as_slice()).unwrap_or(&[]);
        let origin =
            resolve_pbo_from_index(&target, candidates, pbo_prefix(&target).as_deref()).unwrap();
        assert_eq!(origin.id, "111".to_string());
        assert!(!origin.is_pack);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn identity_cache_round_trips() {
        let dir = std::env::temp_dir().join("uksfta-idcache-test");
        fs::create_dir_all(&dir).unwrap();
        // Point the cache at the test dir so we do not touch .uksfta in
        // the workspace; run the round-trip via direct file IO.
        let path = dir.join("identities.json");
        let mut cache: IdentityCache<(String, String)> = HashMap::new();
        cache.insert(
            "TFL".to_string(),
            vec![(
                "3797815099".to_string(),
                "@THE TFL AIO CAG PACK".to_string(),
            )],
        );
        let json = serde_json::to_string_pretty(&cache).unwrap();
        fs::write(&path, json).unwrap();
        let loaded: IdentityCache<(String, String)> =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            loaded.get("TFL").unwrap(),
            &vec![(
                "3797815099".to_string(),
                "@THE TFL AIO CAG PACK".to_string()
            )]
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
