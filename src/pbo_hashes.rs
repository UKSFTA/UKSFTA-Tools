//! Persistent SHA-1 cache for local PBOs.
//!
//! The manifest index confirms a file by its SHA-1, so `identify` and
//! `investigate` would otherwise read the whole pack on every run. A digest
//! is reused only when the canonical path, size and modification time in
//! nanoseconds all match. A changed file is hashed again.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::atomic::write_atomic;
use crate::error::UksftaError;
use crate::workshop_manifest::{sha1_and_size, sha1_from_hex, sha1_hex};

const SCHEMA: u32 = 1;

/// A cold run must not saturate the disk. Four readers are enough to hide
/// latency on an SSD without turning a sequential pack read into a seek
/// storm on a spinning disk.
const MAX_THREADS: usize = 4;

/// A digest and the size it was computed over.
pub type Digest = ([u8; 20], u64);

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Entry {
    size: u64,
    mtime_ns: u64,
    sha1: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    schema: u32,
    entries: BTreeMap<String, Entry>,
}

/// The cache for one run: loaded once, written once.
///
/// One key function serves insert, lookup and removal, so a Windows
/// canonical form cannot be stored under one spelling and looked up under
/// another.
#[derive(Debug)]
pub struct PboHashes {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
    dirty: bool,
    /// Files hashed this run.
    pub hashed: usize,
    /// Digests served from the cache.
    pub reused: usize,
    /// Files that could not be read.
    pub unreadable: usize,
}

impl PboHashes {
    pub fn default_path() -> PathBuf {
        Path::new(".uksfta").join("pbo-hashes.json")
    }

    /// Read the cache. A missing file is an empty cache. A malformed file
    /// is reported once and rebuilt.
    pub fn load(path: &Path) -> Self {
        let entries = match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<CacheFile>(&text) {
                Ok(cache) if cache.schema == SCHEMA => cache.entries,
                Ok(_) => BTreeMap::new(),
                Err(error) => {
                    eprintln!(
                        "Warning: {}: {error}; rebuilding the PBO hash cache",
                        path.display()
                    );
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        Self {
            path: path.to_path_buf(),
            entries,
            dirty: false,
            hashed: 0,
            reused: 0,
            unreadable: 0,
        }
    }

    /// SHA-1 for every path, in input order. A cached digest is used only
    /// when size and modification time still match. Files that cannot be
    /// read yield `None` and their entry is dropped.
    pub fn digests(&mut self, paths: &[PathBuf]) -> Vec<Option<Digest>> {
        let mut out: Vec<Option<Digest>> = vec![None; paths.len()];
        let mut slots: Vec<usize> = Vec::new();
        let mut jobs: Vec<(PathBuf, String, u64, u64)> = Vec::new();

        for (index, path) in paths.iter().enumerate() {
            let key = cache_key(path);
            let Some((size, mtime_ns)) = stamp(path) else {
                self.unreadable += 1;
                self.forget(path);
                continue;
            };
            if let Some(entry) = self.entries.get(&key) {
                if entry.size == size && entry.mtime_ns == mtime_ns {
                    if let Some(digest) = sha1_from_hex(&entry.sha1) {
                        out[index] = Some((digest, size));
                        self.reused += 1;
                        continue;
                    }
                }
            }
            slots.push(index);
            jobs.push((path.clone(), key, size, mtime_ns));
        }

        let computed = hash_parallel(&jobs);
        for ((slot, (_, key, size, mtime_ns)), digest) in slots.into_iter().zip(jobs).zip(computed)
        {
            let Some(digest) = digest else {
                self.unreadable += 1;
                continue;
            };
            out[slot] = Some((digest, size));
            self.entries.insert(
                key,
                Entry {
                    size,
                    mtime_ns,
                    sha1: sha1_hex(&digest),
                },
            );
            self.hashed += 1;
            self.dirty = true;
        }
        out
    }

    /// Drop the entries that could name this path.
    ///
    /// A file that has gone can no longer be canonicalised, so the key is
    /// rebuilt from its parent directory, which still exists. The path as
    /// given is removed too, for an entry written before this rule.
    fn forget(&mut self, path: &Path) -> bool {
        let mut removed = self.entries.remove(&cache_key(path)).is_some();
        removed |= self.entries.remove(&normalise(path)).is_some();
        if removed {
            self.dirty = true;
        }
        removed
    }

    /// Report what the run did and persist the cache. A write failure warns
    /// and leaves the command's result intact.
    pub fn finish(&self) {
        if self.hashed > 0 || self.reused > 0 || self.unreadable > 0 {
            eprintln!(
                "PBO hashes: {} reused, {} hashed, {} unreadable",
                self.reused, self.hashed, self.unreadable
            );
        }
        if !self.dirty {
            return;
        }
        if let Err(error) = self.save() {
            eprintln!(
                "Warning: could not write {}: {error}; the next run hashes again",
                self.path.display()
            );
        }
    }

    fn save(&self) -> Result<(), UksftaError> {
        let cache = CacheFile {
            schema: SCHEMA,
            entries: self.entries.clone(),
        };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(UksftaError::Io)?;
        }
        let json = serde_json::to_vec(&cache)
            .map_err(|error| UksftaError::Parse(format!("PBO hash cache: {error}")))?;
        write_atomic(&self.path, &json)
    }
}

/// The size and mtime in nanoseconds, or `None` when the file cannot be
/// read.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let metadata = fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let mtime_ns = modified.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
    Some((metadata.len(), mtime_ns))
}

/// The one cache key, used for insert, lookup and removal.
///
/// The canonical path resolves symlinks and drive mappings while the file
/// exists. When the file is gone, the canonical parent plus the file name
/// rebuilds the same string, so removal still finds the entry. Windows
/// canonical forms carry a `\\?\` prefix, which is why the key must never
/// be recomputed differently on the way out.
fn cache_key(path: &Path) -> String {
    if let Ok(canonical) = fs::canonicalize(path) {
        return normalise(&canonical);
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        if let Ok(parent) = fs::canonicalize(parent) {
            return normalise(&parent.join(name));
        }
    }
    normalise(path)
}

/// A path as the cache text spells it.
fn normalise(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Hash the jobs with at most [`MAX_THREADS`] readers, preserving order.
fn hash_parallel(jobs: &[(PathBuf, String, u64, u64)]) -> Vec<Option<[u8; 20]>> {
    let mut out: Vec<Option<[u8; 20]>> = vec![None; jobs.len()];
    if jobs.is_empty() {
        return out;
    }
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(MAX_THREADS)
        .min(jobs.len());
    let next = AtomicUsize::new(0);
    let done: Mutex<Vec<(usize, Option<[u8; 20]>)>> = Mutex::new(Vec::with_capacity(jobs.len()));

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                if index >= jobs.len() {
                    break;
                }
                let digest = sha1_and_size(&jobs[index].0).map(|(digest, _)| digest);
                done.lock()
                    .expect("the hash collector is not poisoned")
                    .push((index, digest));
            });
        }
    });

    for (index, digest) in done
        .into_inner()
        .expect("the hash collector is not poisoned")
    {
        out[index] = digest;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn scratch(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("uksfta-hashes-{name}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_unchanged_file_is_hashed_once() {
        let dir = scratch("once");
        let pbo = dir.join("a.pbo");
        fs::write(&pbo, b"pbo bytes").unwrap();
        let cache_path = dir.join("pbo-hashes.json");

        let mut first = PboHashes::load(&cache_path);
        let digest = first.digests(std::slice::from_ref(&pbo));
        assert_eq!(first.hashed, 1);
        assert_eq!(first.reused, 0);
        assert!(digest[0].is_some());
        first.finish();
        assert!(cache_path.is_file());

        let mut second = PboHashes::load(&cache_path);
        assert_eq!(second.digests(std::slice::from_ref(&pbo)), digest);
        assert_eq!(second.hashed, 0);
        assert_eq!(second.reused, 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_changed_mtime_or_size_forces_a_rehash() {
        let dir = scratch("change");
        let pbo = dir.join("a.pbo");
        fs::write(&pbo, b"aaaa").unwrap();
        let cache_path = dir.join("pbo-hashes.json");

        let mut first = PboHashes::load(&cache_path);
        let before = first.digests(std::slice::from_ref(&pbo));
        first.finish();

        // Same length, new content: only the mtime can show the change.
        std::thread::sleep(Duration::from_millis(10));
        fs::write(&pbo, b"bbbb").unwrap();
        let mut changed_mtime = PboHashes::load(&cache_path);
        let after = changed_mtime.digests(std::slice::from_ref(&pbo));
        assert_ne!(after, before);
        assert_eq!(changed_mtime.hashed, 1);
        assert_eq!(changed_mtime.reused, 0);

        // A size change is caught even when the mtime were to match.
        fs::write(&pbo, b"bbbbbb").unwrap();
        let mut changed_size = PboHashes::load(&cache_path);
        let after = changed_size.digests(std::slice::from_ref(&pbo));
        assert_ne!(after, before);
        assert_eq!(changed_size.hashed, 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_file_is_dropped_from_the_cache() {
        let dir = scratch("missing");
        let pbo = dir.join("a.pbo");
        fs::write(&pbo, b"bytes").unwrap();
        let cache_path = dir.join("pbo-hashes.json");
        let mut cache = PboHashes::load(&cache_path);
        cache.digests(std::slice::from_ref(&pbo));
        cache.finish();
        assert_eq!(cache.entries.len(), 1);

        fs::remove_file(&pbo).unwrap();
        let mut cache = PboHashes::load(&cache_path);
        assert_eq!(cache.digests(std::slice::from_ref(&pbo)), vec![None]);
        assert_eq!(cache.unreadable, 1);
        assert!(cache.entries.is_empty(), "the stale entry survived");
        cache.finish();

        // The written text is empty whatever spelling the platform uses
        // for a canonical path.
        let written: CacheFile =
            serde_json::from_str(&fs::read_to_string(&cache_path).unwrap()).unwrap();
        assert!(written.entries.is_empty(), "the stale entry survived");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_path_that_no_longer_canonicalises_is_still_forgotten() {
        let dir = scratch("forget");
        let pbo = dir.join("a.pbo");
        fs::write(&pbo, b"bytes").unwrap();
        let cache_path = dir.join("pbo-hashes.json");
        let mut cache = PboHashes::load(&cache_path);
        cache.digests(std::slice::from_ref(&pbo));
        cache.finish();

        // The entry was keyed while the file existed, in the platform's
        // canonical form. Removal now runs with that form unavailable.
        fs::remove_file(&pbo).unwrap();
        let mut cache = PboHashes::load(&cache_path);
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.forget(&pbo));
        assert!(cache.entries.is_empty());
        assert!(!cache.forget(&pbo), "a second removal reports nothing");
        cache.finish();

        let written: CacheFile =
            serde_json::from_str(&fs::read_to_string(&cache_path).unwrap()).unwrap();
        assert!(written.entries.is_empty(), "the stale entry survived");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_malformed_cache_is_rebuilt() {
        let dir = scratch("malformed");
        let pbo = dir.join("a.pbo");
        fs::write(&pbo, b"bytes").unwrap();
        let cache_path = dir.join("pbo-hashes.json");
        fs::write(&cache_path, "{not json").unwrap();

        let mut cache = PboHashes::load(&cache_path);
        let digest = cache.digests(std::slice::from_ref(&pbo));
        assert!(digest[0].is_some());
        assert_eq!(cache.hashed, 1);
        cache.finish();
        assert!(fs::read_to_string(&cache_path).unwrap().contains("schema"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_thread_cap_is_bounded() {
        const {
            assert!(MAX_THREADS <= 4);
        }
        // More jobs than threads still returns every digest in order.
        let dir = scratch("many");
        let paths: Vec<PathBuf> = (0..9)
            .map(|index| {
                let path = dir.join(format!("{index}.pbo"));
                fs::write(&path, format!("content {index}")).unwrap();
                path
            })
            .collect();
        let mut cache = PboHashes::load(&dir.join("pbo-hashes.json"));
        let digests = cache.digests(&paths);
        assert_eq!(cache.hashed, 9);
        assert_eq!(digests.len(), 9);
        for (path, digest) in paths.iter().zip(digests) {
            let (expected, size) = sha1_and_size(path).unwrap();
            assert_eq!(digest, Some((expected, size)));
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
