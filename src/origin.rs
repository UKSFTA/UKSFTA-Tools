use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::pbo::pbo_prefix;
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

/// The root of an addon prefix: the first path component before a
/// backslash (e.g. "z\ace\addons\grenades" -> "z"). A standalone mod's
/// PBOs share few roots; an aggregate pack spans many.
fn prefix_root(prefix: &str) -> &str {
    prefix.split('\\').next().unwrap_or(prefix)
}

/// The Workshop ID of the current directory, if the current directory is
/// itself a mod folder inside a Workshop cache (e.g. investigating a pack's
/// own contents). Such a folder must not be a candidate for its own PBOs.
pub fn self_workshop_id(caches: &[PathBuf]) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let cwd = fs::canonicalize(&cwd).ok()?;
    for cache in caches {
        let cache = fs::canonicalize(cache).ok()?;
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

/// The local identity cache type: search term -> confirmed (id, title)
/// pairs, persisted to .uksfta/identities.json.
pub type IdentityCache = HashMap<String, Vec<(String, String)>>;

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
    let mut scored: Vec<(&ModDir, u8)> = Vec::new();
    for dir in candidates {
        let cached_path = dir.pbos.iter().find(|pbo| {
            pbo.file_name()
                .map(|n| n == pbo_name.as_str())
                .unwrap_or(false)
        })?;
        let mut score = 0u8;
        if let Some(target) = target_prefix {
            if pbo_prefix(cached_path).as_deref() == Some(target) {
                score += 2; // same canonical addon prefix
            }
        } else {
            // No prefix available: fall back to byte identity.
            let target_hash = file_sha256(pbo_path)?;
            if file_sha256(cached_path) == Some(target_hash) {
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
