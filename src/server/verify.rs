//! The verification phases for the pack: one batch each for the listing,
//! the `meta.cpp`/`mod.cpp` fetch and the deep download.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::config::Settings;
use super::native;
use super::parse;
use super::plan::{
    build_plan, join_remote, mod_dir, LocalPack, Plan, PlanInput, RemoteEntry, RemotePack,
};
use super::sftp::{quote, ScratchDir, Sftp};
use super::signature;
use super::{hash_threads, LocalHashes, MISSING_HASH};
use crate::error::UksftaError;
use crate::util::file_sha256;

/// The most remote key files read in one run.
const MAX_REMOTE_KEYS: usize = 64;

fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}
/// The most directories listed in one run.
const MAX_DIRS: usize = 512;
/// The most directories listed in one batch.
const DIR_BATCH: usize = 32;
/// The deepest directory level listed below the pack root.
const MAX_DEPTH: usize = 8;
/// The deep phase sends nothing, so its local hash takes the full thread cap
/// that [`hash_threads`] allows when no single worker owns the link.
const DEEP_HASH_JOBS: u8 = 2;

/// The reads that need only the listing: the remote key authorities, the
/// deep hash results, and every warning they raise.
struct Fetched {
    authorities: HashSet<String>,
    deep: HashMap<String, bool>,
    warnings: Vec<String>,
}

/// The inputs that stay fixed for one command run.
pub struct Verifier<'a> {
    pub client: Sftp,
    pub settings: &'a Settings,
    pub scratch: &'a ScratchDir,
    pub local: &'a LocalPack,
    pub meta_upload: PathBuf,
    pub locked: &'a HashSet<String>,
    pub deep: bool,
    pub force_prune: bool,
    pub require_signatures: bool,
}

/// The time spent in the read-only phases, in milliseconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct VerifyTiming {
    /// Listing and the identity files. With `--deep` the key read shares the
    /// deep batch, so it is counted in `deep_ms` instead.
    pub connect_ms: u64,
    /// The deep download and hash, when it ran.
    pub deep_ms: Option<u64>,
}

impl Verifier<'_> {
    /// Run every read-only phase and build the plan.
    pub fn verify(&self) -> Result<Plan, UksftaError> {
        self.verify_timed().map(|(plan, _)| plan)
    }

    /// Run the read-only phases and report how long they took.
    pub fn verify_timed(&self) -> Result<(Plan, VerifyTiming), UksftaError> {
        let listing_started = Instant::now();
        let dir = mod_dir(&self.settings.root, &self.local.id);
        let mut remote = self.list(&dir)?;
        self.read_meta(&mut remote);
        let listing_ms = millis(listing_started.elapsed());

        let fetch_started = Instant::now();
        let fetched = self.fetch_keys_and_deep(&dir, &remote)?;
        let fetch_ms = millis(fetch_started.elapsed());
        // The key read shares the deep batch, so with `--deep` its time is
        // the deep phase's.
        let connect_ms = if self.deep {
            listing_ms
        } else {
            listing_ms + fetch_ms
        };
        let deep_ms = self.deep.then_some(fetch_ms);

        let mut warnings = fetched.warnings;
        let mut plan = build_plan(&PlanInput {
            local: self.local,
            remote_dir: &dir,
            remote: &remote,
            meta_upload: Some(&self.meta_upload),
            deep: &fetched.deep,
            locked: self.locked,
            local_signature_authorities: &self.local.signature_authorities,
            local_key_authorities: &self.local.key_authorities,
            remote_key_authorities: &fetched.authorities,
            force_prune: self.force_prune,
            require_signatures: self.require_signatures,
        });
        plan.warnings.append(&mut warnings);
        Ok((
            plan,
            VerifyTiming {
                connect_ms,
                deep_ms,
            },
        ))
    }

    /// One batch fetches the remote keys and, when the deep check runs, every
    /// remote PBO that has a local match. Both reads need only the listing,
    /// so they share one child.
    fn fetch_keys_and_deep(&self, dir: &str, remote: &RemotePack) -> Result<Fetched, UksftaError> {
        let mut warnings = Vec::new();
        let mut names: Vec<&str> = remote.keys().map(|entry| basename(&entry.rel)).collect();
        names.sort_unstable();
        if names.len() > MAX_REMOTE_KEYS {
            warnings.push(format!(
                "keys/: {} key(s); only the first {MAX_REMOTE_KEYS} are read",
                names.len()
            ));
            names.truncate(MAX_REMOTE_KEYS);
        }

        let out_dir = self.scratch.join("remote-keys");
        fs::create_dir_all(&out_dir).map_err(UksftaError::Io)?;
        let mut lines = Vec::new();
        let mut keys = Vec::new();
        for name in names {
            let rel = format!("keys/{name}");
            let target = out_dir.join(name);
            lines.push(format!(
                "-get {} {}",
                quote(&join_remote(dir, &rel))?,
                quote(&target.display().to_string())?
            ));
            keys.push((name.to_string(), target));
        }

        let deep = if self.deep {
            self.deep_hashes(dir, remote)?
        } else {
            HashMap::new()
        };

        if lines.is_empty() {
            return Ok(Fetched {
                authorities: HashSet::new(),
                deep,
                warnings,
            });
        }
        if self.deep {
            // The deep download must succeed, so its failure ends the phase.
            // On that path the key warning is moot.
            if let Err(e) = self.client.batch(&lines) {
                warnings.push(format!("keys/: cannot download the remote keys: {e}"));
                return Err(e);
            }
        } else if let Err(e) = self.client.batch(&lines) {
            warnings.push(format!("keys/: cannot download the remote keys: {e}"));
            return Ok(Fetched {
                authorities: HashSet::new(),
                deep,
                warnings,
            });
        }

        let mut authorities = HashSet::new();
        for (name, path) in keys {
            match signature::read_key_file(&path) {
                Ok(key) => {
                    authorities.insert(key.name);
                }
                Err(e) => warnings.push(format!("keys/{name}: {e}")),
            }
        }

        Ok(Fetched {
            authorities,
            deep,
            warnings,
        })
    }

    /// List the pack root, then every directory below it in bounded batches.
    fn list(&self, dir: &str) -> Result<RemotePack, UksftaError> {
        let mut lines = vec![
            format!("cd {}", quote(&self.settings.root)?),
            format!("-cd {}", quote(dir)?),
            "pwd".to_string(),
            "ls -l".to_string(),
        ];
        lines.extend(self.meta_lines(dir)?);
        let blocks = parse::parse_blocks(&self.client.batch(&lines)?);
        let Some(root_block) = parse::find_block(&blocks, dir) else {
            return Ok(RemotePack::default());
        };

        let mut pack = RemotePack {
            exists: true,
            ..RemotePack::default()
        };
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut seen: HashSet<String> = HashSet::new();
        self.absorb(dir, "", root_block, &mut pack, &mut queue, &mut seen)?;
        for rel in &self.local.dirs {
            self.enqueue(rel, &mut queue, &mut seen)?;
        }

        while !queue.is_empty() {
            let batch: Vec<String> = queue.drain(..queue.len().min(DIR_BATCH)).collect();
            let mut lines = Vec::new();
            for rel in &batch {
                lines.push(format!("-cd {}", quote(&join_remote(dir, rel))?));
                lines.push("pwd".to_string());
                lines.push("ls -l".to_string());
            }
            let blocks = parse::parse_blocks(&self.client.batch(&lines)?);
            for rel in batch {
                let path = join_remote(dir, &rel);
                if let Some(block) = parse::find_block(&blocks, &path) {
                    self.absorb(dir, &rel, block, &mut pack, &mut queue, &mut seen)?;
                }
            }
        }

        pack.files.sort_by(|a, b| a.rel.cmp(&b.rel));
        pack.dirs.sort();
        pack.symlinks.sort();
        Ok(pack)
    }

    fn enqueue(
        &self,
        rel: &str,
        queue: &mut VecDeque<String>,
        seen: &mut HashSet<String>,
    ) -> Result<(), UksftaError> {
        if rel.split('/').count() > MAX_DEPTH || seen.contains(rel) {
            return Ok(());
        }
        if seen.len() >= MAX_DIRS {
            return Err(UksftaError::Input(format!(
                "the pack folder has more than {MAX_DIRS} directories"
            )));
        }
        seen.insert(rel.to_string());
        queue.push_back(rel.to_string());
        Ok(())
    }

    fn absorb(
        &self,
        dir: &str,
        prefix: &str,
        block: &parse::Block,
        pack: &mut RemotePack,
        queue: &mut VecDeque<String>,
        seen: &mut HashSet<String>,
    ) -> Result<(), UksftaError> {
        for item in &block.entries {
            let rel = if prefix.is_empty() {
                item.name.clone()
            } else {
                format!("{prefix}/{}", item.name)
            };
            if item.is_symlink {
                pack.symlinks.push(rel.clone());
                if item.name == "addons" || item.name == "keys" || self.local.dirs.contains(&rel) {
                    return Err(UksftaError::Input(format!(
                        "{} is a symlink; refusing to follow it",
                        join_remote(dir, &rel)
                    )));
                }
                continue;
            }
            if item.is_dir {
                self.enqueue(&rel, queue, seen)?;
                pack.dirs.push(rel);
                continue;
            }
            pack.files.push(RemoteEntry {
                rel: rel.clone(),
                size: item.size,
            });
            if rel == "mod.cpp" {
                pack.mod_size = Some(item.size);
            }
        }
        Ok(())
    }

    /// One batch fetches `meta.cpp` and `mod.cpp` beside the root listing.
    /// `-get` keeps a missing file from aborting the phase, so the fetch does
    /// not wait for the listing to say whether the file is there.
    fn meta_lines(&self, dir: &str) -> Result<Vec<String>, UksftaError> {
        let meta_path = self.scratch.join("remote-meta.cpp");
        let mod_path = self.scratch.join("remote-mod.cpp");
        let _ = fs::remove_file(&meta_path);
        let _ = fs::remove_file(&mod_path);
        Ok(vec![
            format!(
                "-get {} {}",
                quote(&join_remote(dir, "meta.cpp"))?,
                quote(&meta_path.display().to_string())?
            ),
            format!(
                "-get {} {}",
                quote(&join_remote(dir, "mod.cpp"))?,
                quote(&mod_path.display().to_string())?
            ),
        ])
    }

    /// Read the two identity files that the root listing fetched.
    fn read_meta(&self, remote: &mut RemotePack) {
        let meta_path = self.scratch.join("remote-meta.cpp");
        let mod_path = self.scratch.join("remote-mod.cpp");
        if let Ok(text) = fs::read_to_string(&meta_path) {
            let meta = parse::parse_meta_cpp(&text);
            remote.meta_id = meta.publishedid;
        }
        remote.mod_size = fs::metadata(&mod_path)
            .ok()
            .filter(|meta| meta.is_file())
            .map(|meta| meta.len());
    }

    /// Every remote PBO that has a local match is hashed on both sides. The
    /// remote copy streams into the hasher, so no local copy is written. The
    /// local side hashes on its own threads at the same time.
    fn deep_hashes(
        &self,
        dir: &str,
        remote: &RemotePack,
    ) -> Result<HashMap<String, bool>, UksftaError> {
        let remote_by_name: HashMap<String, &RemoteEntry> = remote
            .files
            .iter()
            .map(|entry| (entry.rel.to_ascii_lowercase(), entry))
            .collect();

        let mut targets = Vec::new();
        for pbo in self.local.pbos() {
            let key = pbo.rel.to_ascii_lowercase();
            let Some(remote_file) = remote_by_name.get(&key) else {
                continue;
            };
            targets.push(DeepTarget {
                key,
                local: pbo.path.clone(),
                remote: join_remote(dir, &remote_file.rel),
                size: remote_file.size,
            });
        }
        if targets.is_empty() {
            return Ok(HashMap::new());
        }

        let local = LocalHashes::start(
            targets.iter().map(|target| target.local.clone()).collect(),
            hash_threads(DEEP_HASH_JOBS),
        );
        let paths: Vec<(String, u64)> = targets
            .iter()
            .map(|target| (target.remote.clone(), target.size))
            .collect();
        let remote_hashes = match native::verify::stream_hashes(self.settings, &paths) {
            Ok(streamed) => streamed.digests,
            Err(error) => {
                eprintln!("Warning: deep: {error}; reading the remote PBOs to scratch");
                self.deep_hashes_scratch(&targets)?
            }
        };

        Ok(compare(&targets, &remote_hashes, &local.join()))
    }

    /// The fallback for a setup the native session cannot reach, such as one
    /// that authenticates with a password prompt: download each remote PBO to
    /// scratch, hash it, then unlink it.
    fn deep_hashes_scratch(
        &self,
        targets: &[DeepTarget],
    ) -> Result<Vec<Option<String>>, UksftaError> {
        let out_dir = self.scratch.join("deep");
        fs::create_dir_all(&out_dir).map_err(UksftaError::Io)?;

        let mut lines = Vec::new();
        let mut copies = Vec::new();
        for target in targets {
            let copy = out_dir.join(format!("{}.pbo", target.key.replace('/', "_")));
            lines.push(format!(
                "-get {} {}",
                quote(&target.remote)?,
                quote(&copy.display().to_string())?
            ));
            copies.push(copy);
        }
        self.client.batch(&lines)?;

        let mut digests = Vec::with_capacity(copies.len());
        for copy in copies {
            digests.push(file_sha256(&copy));
            let _ = fs::remove_file(&copy);
        }
        Ok(digests)
    }
}

/// A remote PBO that has a local match, with the path of each copy.
struct DeepTarget {
    key: String,
    local: PathBuf,
    remote: String,
    /// The length the listing reported.
    size: u64,
}

/// A pair matches only when both digests exist and are equal. A digest that
/// is missing is a warning and a mismatch, as a failed download was.
fn compare(
    targets: &[DeepTarget],
    remote: &[Option<String>],
    local: &HashMap<PathBuf, String>,
) -> HashMap<String, bool> {
    let mut results = HashMap::new();
    for (index, target) in targets.iter().enumerate() {
        let local_hash = local
            .get(&target.local)
            .filter(|digest| digest.as_str() != MISSING_HASH);
        let remote_hash = remote.get(index).and_then(Option::as_ref);
        let same = match (local_hash, remote_hash) {
            (Some(a), Some(b)) => a == b,
            _ => {
                eprintln!("Warning: {}: cannot hash the PBO", target.key);
                false
            }
        };
        results.insert(target.key.clone(), same);
    }
    results
}

fn basename(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(key: &str) -> DeepTarget {
        DeepTarget {
            key: key.to_string(),
            local: PathBuf::from(format!("/local/{key}")),
            remote: format!("/remote/{key}"),
            size: 5,
        }
    }

    fn hashed(path: &str, digest: &str) -> HashMap<PathBuf, String> {
        let mut map = HashMap::new();
        map.insert(PathBuf::from(path), digest.to_string());
        map
    }

    #[test]
    fn a_clean_pack_matches_on_every_pair() {
        let targets = [target("a.pbo"), target("b.pbo")];
        let local = [("a.pbo", "11"), ("b.pbo", "22")]
            .iter()
            .flat_map(|(key, digest)| hashed(&format!("/local/{key}"), digest))
            .collect::<HashMap<_, _>>();
        let remote = [Some("11".to_string()), Some("22".to_string())];

        let results = compare(&targets, &remote, &local);

        assert_eq!(results.get("a.pbo"), Some(&true));
        assert_eq!(results.get("b.pbo"), Some(&true));
    }

    #[test]
    fn a_hash_mismatch_is_reported_for_that_pair_only() {
        let targets = [target("a.pbo"), target("b.pbo")];
        let local = [("a.pbo", "11"), ("b.pbo", "22")]
            .iter()
            .flat_map(|(key, digest)| hashed(&format!("/local/{key}"), digest))
            .collect::<HashMap<_, _>>();
        let remote = [Some("11".to_string()), Some("33".to_string())];

        let results = compare(&targets, &remote, &local);

        assert_eq!(results.get("a.pbo"), Some(&true));
        assert_eq!(results.get("b.pbo"), Some(&false));
    }

    #[test]
    fn a_remote_read_error_is_a_mismatch_for_every_affected_pair() {
        let local = [("a.pbo", "11"), ("b.pbo", "22")]
            .iter()
            .flat_map(|(key, digest)| hashed(&format!("/local/{key}"), digest))
            .collect::<HashMap<_, _>>();
        // The second remote read failed, and a third pair has no digest at all.
        let remote = [Some("11".to_string()), None];

        let results = compare(
            &[target("a.pbo"), target("b.pbo"), target("c.pbo")],
            &remote,
            &local,
        );

        assert_eq!(results.get("a.pbo"), Some(&true));
        assert_eq!(results.get("b.pbo"), Some(&false));
        assert_eq!(results.get("c.pbo"), Some(&false));
    }

    #[test]
    fn a_local_file_that_cannot_be_hashed_is_a_mismatch() {
        let targets = [target("a.pbo")];
        let local = hashed("/local/a.pbo", MISSING_HASH);
        let remote = [Some("11".to_string())];

        let results = compare(&targets, &remote, &local);

        assert_eq!(results.get("a.pbo"), Some(&false));
    }

    #[test]
    fn every_target_reaches_the_plan_even_with_no_digest() {
        let targets = [target("a.pbo"), target("b.pbo")];

        let results = compare(&targets, &[], &HashMap::new());

        assert_eq!(results.len(), 2);
        assert!(results.values().all(|same| !same));
    }
}
