use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::pbo::pbo_prefix;
use crate::prefix::prefix_root;
use crate::util::{file_sha256, find_pbos};

/// One cached PBO with the header prefix read from it once, when the
/// folder was scanned.
///
/// A common name such as `ace_common.pbo` sits in dozens of packs, so
/// every target resolved by that name used to re-open the same files and
/// re-read the same headers. The prefix is read here and matched from
/// memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedPbo {
    pub path: PathBuf,
    pub prefix: Option<String>,
}

/// A Workshop mod folder plus precomputed statistics used to arbitrate
/// origin: how many PBOs it holds and how many distinct prefix roots
/// (a standalone mod has few roots; an aggregate pack spans many).
pub struct ModDir {
    pub id: String,
    pub path: PathBuf,
    pub pbos: Vec<CachedPbo>,
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
                        let pbos: Vec<CachedPbo> = find_pbos(&path)
                            .into_iter()
                            .map(|pbo| CachedPbo {
                                prefix: pbo_prefix(&pbo),
                                path: pbo,
                            })
                            .collect();
                        let root_count = {
                            let mut roots = std::collections::HashSet::new();
                            for pbo in &pbos {
                                if let Some(prefix) = &pbo.prefix {
                                    roots.insert(prefix_root(prefix).to_lowercase());
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

/// Single-pass index of every cached PBO name to the copies of that name
/// and the folders that hold them.
///
/// One name maps to many copies, so a target PBO resolves without a scan
/// of any folder's PBO list.
pub struct PboIndex<'a> {
    by_name: HashMap<String, Vec<(&'a ModDir, &'a CachedPbo)>>,
}

impl<'a> PboIndex<'a> {
    /// The cached copies of `name`, in cache walk order. Empty when no
    /// folder holds the name.
    pub fn candidates(&self, name: &str) -> &[(&'a ModDir, &'a CachedPbo)] {
        self.by_name.get(name).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Build the per-name index. This is the expensive cache walk; resolving
/// then becomes in-memory lookups instead of re-scanning folders per PBO.
///
/// A folder that holds the same name twice contributes its first copy
/// only, which is the copy a linear scan would have scored.
pub fn build_pbo_index(mod_dirs: &[ModDir]) -> PboIndex<'_> {
    let mut by_name: HashMap<String, Vec<(&ModDir, &CachedPbo)>> = HashMap::new();
    for dir in mod_dirs {
        let mut seen: HashSet<&str> = HashSet::new();
        for pbo in &dir.pbos {
            if let Some(name) = pbo.path.file_name().and_then(|n| n.to_str()) {
                if seen.insert(name) {
                    by_name
                        .entry(name.to_string())
                        .or_default()
                        .push((dir, pbo));
                }
            }
        }
    }
    PboIndex { by_name }
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
/// `candidates` is the prebuilt per-name copy list from the index, each
/// with its prefix already read.
///
/// Set `UKSFTA_ORIGIN_TRACE` to report every byte-hash fallback on
/// stderr, with the target size and the number of copies it must hash.
pub fn resolve_pbo_from_index(
    pbo_path: &Path,
    candidates: &[(&ModDir, &CachedPbo)],
    target_prefix: Option<&str>,
) -> Option<ResolvedOrigin> {
    // Score each candidate. Prefix match is the primary signal and costs
    // nothing here (the header was read when the folder was scanned).
    // Byte-hash is a fallback used only when the prefix path fails, since
    // hashing reads the whole PBO. The target hash is computed once: a
    // single unreadable target leaves every byte-hash candidate unscored
    // but does not abort.
    let target_hash = match target_prefix {
        Some(_) => None,
        None => {
            trace_fallback(pbo_path, candidates.len());
            Some(file_sha256(pbo_path)?)
        }
    };
    let mut scored: Vec<(&ModDir, u8)> = Vec::new();
    for &(dir, cached) in candidates {
        let mut score = 0u8;
        if let Some(target) = target_prefix {
            if cached.prefix.as_deref() == Some(target) {
                score += 2; // same canonical addon prefix
            }
        } else if file_sha256(&cached.path) == target_hash {
            // No prefix available: fall back to byte identity.
            score += 1;
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

/// Report a byte-hash fallback when `UKSFTA_ORIGIN_TRACE` is set.
///
/// The fallback hashes whole files, so how often it runs and how large
/// the files are decides what it costs. It is a report, never a change to
/// the result.
fn trace_fallback(pbo_path: &Path, copies: usize) {
    if std::env::var_os("UKSFTA_ORIGIN_TRACE").is_none() {
        return;
    }
    let name = pbo_path.file_name().map(|n| n.to_string_lossy());
    let bytes = fs::metadata(pbo_path).map(|meta| meta.len()).unwrap_or(0);
    eprintln!(
        "origin: no prefix in {} ({bytes} bytes); byte-hashing {copies} cached copies",
        name.as_deref().unwrap_or_default()
    );
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

    /// The cached PBO list a folder scan would produce.
    fn cached(dir: &Path) -> Vec<CachedPbo> {
        find_pbos(dir)
            .into_iter()
            .map(|path| CachedPbo {
                prefix: pbo_prefix(&path),
                path,
            })
            .collect()
    }

    /// The old algorithm: for each folder, find the first copy by name and
    /// re-read its header from disk. The cached lookup must agree with it.
    fn rescan(
        mod_dirs: &[ModDir],
        target: &Path,
        target_prefix: Option<&str>,
    ) -> Option<ResolvedOrigin> {
        let name = target.file_name()?.to_str()?.to_string();
        let target_hash = match target_prefix {
            Some(_) => None,
            None => Some(file_sha256(target)?),
        };
        let mut scored: Vec<(&ModDir, u8)> = Vec::new();
        for dir in mod_dirs {
            let Some(path) = dir
                .pbos
                .iter()
                .map(|pbo| &pbo.path)
                .find(|path| path.file_name().and_then(|n| n.to_str()) == Some(name.as_str()))
            else {
                continue;
            };
            let score = match target_prefix {
                Some(prefix) => u8::from(pbo_prefix(path).as_deref() == Some(prefix)) * 2,
                None => u8::from(file_sha256(path) == target_hash),
            };
            if score > 0 {
                scored.push((dir, score));
            }
        }
        let best_score = scored.iter().map(|(_, score)| *score).max()?;
        let best = scored
            .into_iter()
            .filter(|(_, score)| *score == best_score)
            .min_by_key(|(dir, _)| dir.root_count)?
            .0;
        Some(ResolvedOrigin {
            id: best.id.clone(),
            prefix: target_prefix.map(str::to_string),
            is_pack: best.root_count > 50,
        })
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
                pbos: cached(&dir.join("222")),
                root_count: 50,
            },
            ModDir {
                id: "111".to_string(),
                path: dir.join("111"),
                pbos: cached(&dir.join("111")),
                root_count: 1,
            },
        ];
        let index = build_pbo_index(&mod_dirs);

        let target = dir.join("common.pbo");
        fs::write(&target, content).unwrap();

        // Arbitration must pick the standalone mod (111) with fewer roots
        let candidates = index.candidates("common.pbo");
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
            pbos: cached(&dir.join("111")),
            root_count: 1,
        }];
        let index = build_pbo_index(&mod_dirs);
        let candidates = index.candidates("target.pbo");
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
                pbos: cached(&dir.join("222")),
                root_count: 50,
            },
            ModDir {
                id: "111".to_string(),
                path: dir.join("111"),
                pbos: cached(&dir.join("111")),
                root_count: 1,
            },
        ];
        let index = build_pbo_index(&mod_dirs);

        let target = dir.join("common.pbo");
        write_pbo(&target, "z\\ace\\addons\\common", b"version-a");

        let candidates = index.candidates("common.pbo");
        let origin =
            resolve_pbo_from_index(&target, candidates, pbo_prefix(&target).as_deref()).unwrap();
        assert_eq!(origin.id, "111".to_string());
        assert!(!origin.is_pack);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cached_prefix_lookup_equals_a_full_rescan() {
        // One name in several packs, as ace_common.pbo is in reality. The
        // cached-prefix lookup must reach the same folder as the rescan
        // that re-reads every header.
        let dir = std::env::temp_dir().join("uksfta-origin-index");
        fs::remove_dir_all(&dir).ok();
        let standalone = dir.join("111").join("addons");
        let pack_a = dir.join("222").join("addons");
        let pack_b = dir.join("333").join("addons");
        let targets = dir.join("targets");
        for path in [&standalone, &pack_a, &pack_b, &targets] {
            fs::create_dir_all(path).unwrap();
        }

        write_pbo(
            &standalone.join("common.pbo"),
            "z\\ace\\addons\\common",
            b"a",
        );
        write_pbo(
            &standalone.join("ace_common.pbo"),
            "z\\ace\\addons\\main",
            b"a",
        );

        write_pbo(
            &pack_a.join("common.pbo"),
            "z\\ace\\addons\\common",
            b"pack-a",
        );
        write_pbo(
            &pack_a.join("ace_common.pbo"),
            "z\\ace\\addons\\main",
            b"pack-a",
        );
        write_pbo(
            &pack_a.join("loose.pbo"),
            "z\\misc\\addons\\loose",
            b"pack-a",
        );

        write_pbo(
            &pack_b.join("common.pbo"),
            "z\\other\\addons\\common",
            b"pack-b",
        );
        // A second copy of the same name inside one folder: the first in
        // walk order wins, in the index and in the scan alike.
        fs::create_dir_all(pack_b.join("nested")).unwrap();
        write_pbo(
            &pack_b.join("nested").join("ace_common.pbo"),
            "z\\ace\\addons\\main",
            b"pack-b",
        );

        let mod_dirs = vec![
            ModDir {
                id: "222".to_string(),
                path: dir.join("222"),
                pbos: cached(&dir.join("222")),
                root_count: 61,
            },
            ModDir {
                id: "333".to_string(),
                path: dir.join("333"),
                pbos: cached(&dir.join("333")),
                root_count: 61,
            },
            ModDir {
                id: "111".to_string(),
                path: dir.join("111"),
                pbos: cached(&dir.join("111")),
                root_count: 1,
            },
        ];
        let index = build_pbo_index(&mod_dirs);

        // Prefix targets, then a target with no prefix that only bytes
        // can match, then a target that matches nothing. Each is probed
        // with its prefix and again with no prefix, so the byte-hash
        // fallback is compared too.
        let ace_common = "z\\ace\\addons\\common";
        let ace_main = "z\\ace\\addons\\main";
        let cases: [(&str, Option<&str>, &[u8]); 5] = [
            ("common.pbo", Some(ace_common), b"a"),
            ("ace_common.pbo", Some(ace_main), b"a"),
            ("loose.pbo", Some("z\\misc\\addons\\loose"), b"pack-a"),
            ("absent.pbo", Some(ace_common), b"x"),
            ("common.pbo", Some("z\\nowhere\\addons\\common"), b"x"),
        ];
        for (name, prefix, payload) in cases {
            let target = targets.join(name);
            write_pbo(&target, prefix.unwrap_or(ace_main), payload);
            for probe in [prefix, None] {
                assert_eq!(
                    resolve_pbo_from_index(&target, index.candidates(name), probe),
                    rescan(&mod_dirs, &target, probe),
                    "{name} with prefix {probe:?}"
                );
            }
        }

        assert_eq!(
            resolve_pbo_from_index(
                &targets.join("common.pbo"),
                index.candidates("common.pbo"),
                Some(ace_common)
            )
            .map(|origin| origin.id),
            Some("111".to_string())
        );
        assert_eq!(
            resolve_pbo_from_index(
                &targets.join("loose.pbo"),
                index.candidates("loose.pbo"),
                None
            )
            .map(|origin| (origin.id, origin.is_pack)),
            Some(("222".to_string(), true))
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cached_prefix_matches_without_reading_the_file() {
        // The prefix is stored at build time, so a match needs no file
        // access. A path that does not exist proves the point: a rescan
        // finds nothing, the cached lookup finds the folder.
        let dir = std::env::temp_dir().join("uksfta-origin-cached");
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("111").join("addons").join("common.pbo");
        let mod_dirs = vec![ModDir {
            id: "111".to_string(),
            path: dir.join("111"),
            pbos: vec![CachedPbo {
                path: missing,
                prefix: Some("z\\ace\\addons\\common".to_string()),
            }],
            root_count: 1,
        }];
        let index = build_pbo_index(&mod_dirs);

        let outcome = resolve_pbo_from_index(
            &dir.join("common.pbo"),
            index.candidates("common.pbo"),
            Some("z\\ace\\addons\\common"),
        );
        assert_eq!(outcome.map(|origin| origin.id), Some("111".to_string()));
        assert_eq!(
            rescan(
                &mod_dirs,
                &dir.join("common.pbo"),
                Some("z\\ace\\addons\\common")
            ),
            None
        );

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
