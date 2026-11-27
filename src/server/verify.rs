//! The verification phases for the pack: one batch each for the listing,
//! the `meta.cpp`/`mod.cpp` fetch and the deep download.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::config::Settings;
use super::parse;
use super::plan::{
    build_plan, join_remote, mod_dir, LocalPack, Plan, PlanInput, RemoteEntry, RemotePack,
};
use super::sftp::{quote, ScratchDir, Sftp};
use super::signature;
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

        let deep_dir = self.scratch.join("deep");
        let mut targets = Vec::new();
        if self.deep {
            fs::create_dir_all(&deep_dir).map_err(UksftaError::Io)?;
            let remote_by_name: HashMap<String, &RemoteEntry> = remote
                .files
                .iter()
                .map(|entry| (entry.rel.to_ascii_lowercase(), entry))
                .collect();
            for pbo in self.local.pbos() {
                let key = pbo.rel.to_ascii_lowercase();
                let Some(remote_file) = remote_by_name.get(&key) else {
                    continue;
                };
                let target = deep_dir.join(format!("{}.pbo", key.replace('/', "_")));
                lines.push(format!(
                    "-get {} {}",
                    quote(&join_remote(dir, &remote_file.rel))?,
                    quote(&target.display().to_string())?
                ));
                targets.push((key, pbo.path.clone(), target));
            }
        }

        if lines.is_empty() {
            return Ok(Fetched {
                authorities: HashSet::new(),
                deep: HashMap::new(),
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
                deep: HashMap::new(),
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

        let mut deep = HashMap::new();
        for (key, local, downloaded) in targets {
            let same = match (file_sha256(&local), file_sha256(&downloaded)) {
                (Some(a), Some(b)) => a == b,
                _ => {
                    eprintln!("Warning: {key}: cannot hash the downloaded PBO");
                    false
                }
            };
            let _ = fs::remove_file(&downloaded);
            deep.insert(key, same);
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
}

fn basename(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}
