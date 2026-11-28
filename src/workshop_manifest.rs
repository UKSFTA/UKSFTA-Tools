//! Steam depot manifest index for exact PBO origin.
//!
//! Steam writes the content manifest of every installed Workshop item to
//! `<library>/depotcache/<appid>_<manifestid>.manifest`. Each manifest lists
//! every file of the item with its size and the SHA-1 of its contents. The
//! index joins `appworkshop_<appid>.acf` (published id -> manifest id) with
//! those manifests, so the SHA-1 of a PBO confirms which item published
//! those exact bytes under which path.
//!
//! The container is little-endian:
//!
//! ```text
//! u32 0x71F617D0  payload length  payload
//! u32 0x1F4812BE  metadata length metadata
//! u32 0x1B81B817  signature length signature
//! u32 0x32C415AB
//! ```
//!
//! The payload is a protobuf message with repeated `FileMapping` in field 1.
//! `FileMapping` carries 1 filename, 2 size, 3 flags (64 = directory),
//! 4 sha_filename, 5 sha_content, 6 chunks, 7 linktarget. Only fields 1, 2,
//! 3, 5 and 7 are read: the rest of the container is validated and skipped.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::atomic::write_atomic;
use crate::steam::{find_all_workshop_caches, find_steam_library_folders, parse_acf, STEAM_APP_ID};

const MANIFEST_MAGIC: u32 = 0x71F6_17D0;
const PAYLOAD_END_MAGIC: u32 = 0x1F48_12BE;
const METADATA_END_MAGIC: u32 = 0x1B81_B817;
const SIGNATURE_END_MAGIC: u32 = 0x32C4_15AB;

/// The `FileMapping` flag that marks a directory entry.
const FLAG_DIRECTORY: u64 = 0x40;

/// Schema of `.uksfta/workshop-index.json`. A different value is discarded
/// and rebuilt, never converted.
const CACHE_SCHEMA: u32 = 2;

/// The legacy manifest value: the item predates manifests and cannot be
/// indexed.
pub const LEGACY_MANIFEST: &str = "-1";

// --- Container and protobuf parsing ---------------------------------------

/// Why a manifest could not be read. Every variant means the file is
/// ignored, not that the command fails.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("manifest ends before the field at offset {0}")]
    Truncated(usize),
    #[error("bad magic {got:#010x} at offset {offset}, expected {expected:#010x}")]
    Magic {
        offset: usize,
        got: u32,
        expected: u32,
    },
    #[error("varint at offset {0} does not fit 64 bits")]
    Varint(usize),
    #[error("protobuf field {field} uses unsupported wire type {wire}")]
    Wire { field: u64, wire: u64 },
    #[error("{0} trailing byte(s) after the closing magic")]
    Trailing(usize),
}

/// One file in a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    pub flags: u64,
    pub sha_content: [u8; 20],
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], ManifestError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(ManifestError::Truncated(self.pos))?;
        if end > self.data.len() {
            return Err(ManifestError::Truncated(self.pos));
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u32_le(&mut self) -> Result<u32, ManifestError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn varint(&mut self) -> Result<u64, ManifestError> {
        let start = self.pos;
        let mut value = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = *self
                .data
                .get(self.pos)
                .ok_or(ManifestError::Truncated(self.pos))?;
            self.pos += 1;
            value |= u64::from(byte & 0x7f)
                .checked_shl(shift)
                .ok_or(ManifestError::Varint(start))?;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
            shift += 7;
            if shift >= 64 {
                return Err(ManifestError::Varint(start));
            }
        }
    }

    /// Read a length-delimited field: varint length, then that many bytes.
    fn delimited(&mut self) -> Result<&'a [u8], ManifestError> {
        let len = self.varint()?;
        let len = usize::try_from(len).map_err(|_| ManifestError::Truncated(self.pos))?;
        self.take(len)
    }
}

fn skip_field(reader: &mut Reader<'_>, field: u64, wire: u64) -> Result<(), ManifestError> {
    match wire {
        0 => {
            reader.varint()?;
        }
        1 => {
            reader.take(8)?;
        }
        2 => {
            reader.delimited()?;
        }
        5 => {
            reader.take(4)?;
        }
        _ => return Err(ManifestError::Wire { field, wire }),
    }
    Ok(())
}

/// Parse one `FileMapping`. A directory or a link target is not a file with
/// content, so it yields `None`. A mapping without a 20-byte content hash is
/// also ignored: it cannot identify anything.
fn parse_file_mapping(data: &[u8]) -> Result<Option<ManifestFile>, ManifestError> {
    let mut reader = Reader::new(data);
    let mut path: Option<String> = None;
    let mut size = 0u64;
    let mut flags = 0u64;
    let mut sha_content: Option<[u8; 20]> = None;
    let mut is_link = false;

    while reader.remaining() > 0 {
        let key = reader.varint()?;
        let field = key >> 3;
        let wire = key & 7;
        match (field, wire) {
            (1, 2) => {
                path = Some(String::from_utf8_lossy(reader.delimited()?).into_owned());
            }
            (2, 0) => size = reader.varint()?,
            (3, 0) => flags = reader.varint()?,
            (5, 2) => {
                let raw = reader.delimited()?;
                if let Ok(digest) = <[u8; 20]>::try_from(raw) {
                    sha_content = Some(digest);
                }
            }
            (7, 2) => {
                reader.delimited()?;
                is_link = true;
            }
            _ => skip_field(&mut reader, field, wire)?,
        }
    }

    if is_link || flags & FLAG_DIRECTORY != 0 {
        return Ok(None);
    }
    let (Some(path), Some(sha_content)) = (path, sha_content) else {
        return Ok(None);
    };
    Ok(Some(ManifestFile {
        path,
        size,
        flags,
        sha_content,
    }))
}

fn parse_payload(payload: &[u8]) -> Result<Vec<ManifestFile>, ManifestError> {
    let mut reader = Reader::new(payload);
    let mut files = Vec::new();
    while reader.remaining() > 0 {
        let key = reader.varint()?;
        let field = key >> 3;
        let wire = key & 7;
        if (field, wire) == (1, 2) {
            if let Some(file) = parse_file_mapping(reader.delimited()?)? {
                files.push(file);
            }
        } else {
            skip_field(&mut reader, field, wire)?;
        }
    }
    Ok(files)
}

/// Parse one depot manifest. The whole container is validated, so a
/// truncated or spliced file is rejected rather than half-read.
pub fn parse_manifest(data: &[u8]) -> Result<Vec<ManifestFile>, ManifestError> {
    let mut reader = Reader::new(data);
    let magic = reader.u32_le()?;
    if magic != MANIFEST_MAGIC {
        return Err(ManifestError::Magic {
            offset: 0,
            got: magic,
            expected: MANIFEST_MAGIC,
        });
    }
    let payload_len = reader.u32_le()? as usize;
    let payload = reader.take(payload_len)?;

    for expected in [PAYLOAD_END_MAGIC, METADATA_END_MAGIC, SIGNATURE_END_MAGIC] {
        let offset = reader.pos;
        let got = reader.u32_le()?;
        if got != expected {
            return Err(ManifestError::Magic {
                offset,
                got,
                expected,
            });
        }
        if expected != SIGNATURE_END_MAGIC {
            let len = reader.u32_le()? as usize;
            reader.take(len)?;
        }
    }

    if reader.remaining() != 0 {
        return Err(ManifestError::Trailing(reader.remaining()));
    }
    parse_payload(payload)
}

// --- Depot cache discovery ------------------------------------------------

/// Every `depotcache` directory Steam may use. Each library root keeps its
/// own, plus the Steam root itself.
pub fn find_depot_caches() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for steamapps in find_steam_library_folders() {
        let Some(root) = steamapps.parent() else {
            continue;
        };
        let cache = root.join("depotcache");
        if cache.is_dir() && !dirs.contains(&cache) {
            dirs.push(cache);
        }
    }
    dirs
}

fn acf_paths() -> Vec<PathBuf> {
    let mut seen = std::collections::BTreeSet::new();
    find_all_workshop_caches()
        .into_iter()
        .filter_map(|cache| {
            let workshop = cache.parent()?.parent()?;
            Some(workshop.join(format!("appworkshop_{}.acf", STEAM_APP_ID)))
        })
        .filter(|path| path.is_file())
        // Two library folders can name the same ACF through a symlink.
        .filter_map(|path| fs::canonicalize(path).ok())
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// The manifest file for one manifest id, in the first cache that holds it.
fn manifest_path(caches: &[PathBuf], manifest_id: &str) -> Option<PathBuf> {
    let name = format!("{STEAM_APP_ID}_{manifest_id}.manifest");
    caches
        .iter()
        .map(|cache| cache.join(&name))
        .find(|path| path.is_file())
}

// --- Index ----------------------------------------------------------------

/// One item that holds a file with the requested bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub published_id: String,
    pub path: String,
    pub size: u64,
}

/// Items that publish a PBO basename. `exact_size` is true when every entry
/// matched the requested size; false when the size narrowed nothing and
/// every item with the name is returned.
#[derive(Debug, Default)]
pub struct NameCandidates<'a> {
    pub entries: Vec<&'a IndexEntry>,
    pub exact_size: bool,
}

impl<'a> NameCandidates<'a> {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The distinct item ids, in index order. More than one id means the
    /// name alone cannot decide the origin.
    pub fn ids(&self) -> Vec<&'a str> {
        let mut ids: Vec<&str> = self
            .entries
            .iter()
            .map(|e| e.published_id.as_str())
            .collect();
        ids.dedup();
        ids
    }
}

/// How many distinct items publish a matched file.
pub fn distinct_items(entries: &[IndexEntry]) -> usize {
    let mut ids: Vec<&str> = entries.iter().map(|e| e.published_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.len()
}

/// The report suffix for a match that more than one item publishes.
pub fn shared_suffix(entries: &[IndexEntry]) -> String {
    let count = distinct_items(entries);
    if count > 1 {
        format!(", shared by {} items", count)
    } else {
        String::new()
    }
}

/// A content-addressed index of every file in every installed manifest,
/// with a basename index over the PBOs for discovery before hashing.
#[derive(Debug, Default)]
pub struct WorkshopIndex {
    by_content: HashMap<([u8; 20], u64), Vec<IndexEntry>>,
    /// Lowercased PBO basename -> entries, ordered like the content index.
    by_name: HashMap<String, Vec<IndexEntry>>,
    /// Items with a manifest, counted once each.
    pub manifests: usize,
    /// Items whose ACF manifest is `-1`; they predate manifests.
    pub legacy: usize,
    /// Items whose manifest file is absent from every depot cache.
    pub missing: usize,
    /// Manifest files rejected as unreadable.
    pub broken: usize,
    /// Regular files in the index.
    pub files: usize,
    /// PBO entries reachable by name.
    pub pbo_entries: usize,
    /// Distinct PBO basenames.
    pub names: usize,
    /// Manifests parsed this run rather than read from the cache.
    pub rebuilt: usize,
    /// Time spent building the index, including any manifest reads.
    pub elapsed: std::time::Duration,
}

impl WorkshopIndex {
    /// The index for a command. `None` means it is absent or has no
    /// entries, and the caller keeps its existing behaviour.
    pub fn load() -> Option<Self> {
        load_index()
    }

    pub fn is_empty(&self) -> bool {
        self.files == 0
    }

    /// Every item holding a file with exactly these bytes. Ordered by the
    /// fewest files in the item first, so a standalone mod comes before an
    /// aggregate pack that bundled a copy.
    pub fn lookup(&self, sha: &[u8; 20], size: u64) -> &[IndexEntry] {
        self.by_content
            .get(&(*sha, size))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Remove every entry published by `id`, and keep the counters truthful.
    /// A caller that investigates one item must not resolve that item's
    /// files back to itself, and it learns the id only after the index is
    /// built.
    pub fn exclude_item(&mut self, id: &str) {
        let mut removed = 0usize;
        self.by_content.retain(|_, entries| {
            let before = entries.len();
            entries.retain(|entry| entry.published_id != id);
            removed += before - entries.len();
            !entries.is_empty()
        });
        self.by_name.retain(|_, entries| {
            entries.retain(|entry| entry.published_id != id);
            !entries.is_empty()
        });
        if removed > 0 {
            self.files = self.files.saturating_sub(removed);
            self.manifests = self.manifests.saturating_sub(1);
            self.pbo_entries = self.by_name.values().map(Vec::len).sum();
            self.names = self.by_name.len();
        }
    }

    /// The item to report for a match. A caller with a prefix-derived id
    /// passes it as a hint: when the same bytes are in several items, the
    /// item the prefix named wins. Otherwise the item with the fewest files
    /// wins, which is the standalone mod over any pack that bundled it.
    pub fn pick<'a>(
        &self,
        entries: &'a [IndexEntry],
        hint: Option<&str>,
    ) -> Option<&'a IndexEntry> {
        if let Some(hint) = hint {
            if let Some(entry) = entries.iter().find(|entry| entry.published_id == hint) {
                return Some(entry);
            }
        }
        entries.first()
    }

    /// Items publishing this PBO basename. A size that matches at least one
    /// entry narrows the list; otherwise every item with the name is
    /// returned and `exact_size` is false.
    pub fn candidates(&self, name: &str, size: u64) -> NameCandidates<'_> {
        let Some(all) = self.by_name.get(&name.to_ascii_lowercase()) else {
            return NameCandidates::default();
        };
        let exact: Vec<&IndexEntry> = all.iter().filter(|entry| entry.size == size).collect();
        if exact.is_empty() {
            NameCandidates {
                entries: all.iter().collect(),
                exact_size: false,
            }
        } else {
            NameCandidates {
                entries: exact,
                exact_size: true,
            }
        }
    }

    /// The entry `id` publishes for a file called `name`. A local PBO is
    /// compared with this one to say whether it is the installed version.
    pub fn installed_entry(&self, id: &str, name: &str) -> Option<&IndexEntry> {
        let entries = self.by_name.get(&name.to_ascii_lowercase())?;
        entries.iter().find(|entry| {
            entry.published_id == id && path_basename(&entry.path).eq_ignore_ascii_case(name)
        })
    }

    /// Candidate discovery for one file: its basename and its size.
    pub fn candidates_for(&self, path: &Path) -> NameCandidates<'_> {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return NameCandidates::default();
        };
        let Some(all) = self.by_name.get(&name.to_ascii_lowercase()) else {
            return NameCandidates::default();
        };
        match fs::metadata(path) {
            Ok(metadata) => self.candidates(name, metadata.len()),
            Err(_) => NameCandidates {
                entries: all.iter().collect(),
                exact_size: false,
            },
        }
    }
}

/// The last path component of a manifest path, either separator.
fn path_basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Where archived manifests live: a per-user cache, never the current
/// directory, so every pack folder does not keep its own copy.
pub fn archive_dir() -> PathBuf {
    dirs::cache_dir()
        .map(|dir| dir.join("uksfta").join("manifests"))
        .unwrap_or_else(|| std::env::temp_dir().join("uksfta-manifests"))
}

/// Keep one copy of an item's manifest. Steam deletes the old manifest on
/// update, so this is the only way a later run can still identify the
/// previous version's bytes.
fn archive_manifest(source: &Path, published_id: &str, manifest_id: &str) {
    let dir = archive_dir();
    let target = dir.join(format!("{published_id}_{manifest_id}.manifest"));
    if target.exists() {
        return;
    }
    let copied = fs::create_dir_all(&dir).and_then(|()| fs::copy(source, &target).map(drop));
    if let Err(error) = copied {
        eprintln!("Warning: could not archive {}: {error}", source.display());
    }
}

/// Every archived manifest as `(item id, manifest id, path)`.
fn archived_manifests() -> Vec<(String, String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir(archive_dir()) else {
        return out;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".manifest") else {
            continue;
        };
        let Some((id, manifest)) = stem.split_once('_') else {
            continue;
        };
        out.push((id.to_string(), manifest.to_string(), path));
    }
    out.sort();
    out
}

/// Parse one manifest file, through the cache. `rebuilt` counts a parse,
/// `broken` a file that could not be used, and `dirty` records a new cache
/// entry worth writing.
fn load_manifest_files(
    path: &Path,
    manifest_id: &str,
    cache: &mut IndexCache,
    rebuilt: &mut usize,
    broken: &mut usize,
    dirty: &mut bool,
) -> Option<Vec<ManifestFile>> {
    let stamp = FileStamp::read(path);
    if let Some(cached) = cache.manifests.get(manifest_id) {
        if Some(cached.stamp) == stamp {
            return Some(
                cached
                    .files
                    .iter()
                    .filter_map(|file| file.to_file())
                    .collect(),
            );
        }
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!(
                "Warning: {}: {error}; ignoring this manifest",
                path.display()
            );
            *broken += 1;
            return None;
        }
    };
    match parse_manifest(&bytes) {
        Ok(files) => {
            *rebuilt += 1;
            if let Some(stamp) = stamp {
                cache.manifests.insert(
                    manifest_id.to_string(),
                    CachedManifest {
                        stamp,
                        files: files.iter().map(CachedFile::from_file).collect(),
                    },
                );
                *dirty = true;
            }
            Some(files)
        }
        Err(error) => {
            eprintln!(
                "Warning: {}: {error}; ignoring this manifest",
                path.display()
            );
            *broken += 1;
            None
        }
    }
}

/// Build the index from every installed Workshop item.
///
/// No failure reaches the caller. An absent Steam install yields an empty
/// index; an unreadable manifest is reported once and skipped; a cache that
/// cannot be written costs only the parse next run.
pub fn build_index(depot_caches: &[PathBuf], cache_path: &Path) -> WorkshopIndex {
    build_from(&acf_paths(), depot_caches, cache_path)
}

/// Build from explicit ACF paths. Tests use this; the real path discovers
/// them from Steam.
pub fn build_from(acfs: &[PathBuf], depot_caches: &[PathBuf], cache_path: &Path) -> WorkshopIndex {
    let started = std::time::Instant::now();
    let mut stamps = BTreeMap::new();
    let mut published: Vec<(String, String)> = Vec::new();
    let mut legacy = 0usize;

    for acf in acfs {
        if let Some(stamp) = FileStamp::read(acf) {
            stamps.insert(acf.display().to_string(), stamp);
        }
        let Ok(text) = fs::read_to_string(acf) else {
            eprintln!("Warning: could not read {}; skipping it", acf.display());
            continue;
        };
        for (id, item) in parse_acf(&text) {
            if item.manifest.is_empty() {
                continue;
            }
            if item.manifest == LEGACY_MANIFEST {
                legacy += 1;
                continue;
            }
            published.push((id, item.manifest));
        }
    }

    published.sort_by(|left, right| numeric_key(&left.0).cmp(&numeric_key(&right.0)));

    let mut cache = load_cache(cache_path, &stamps).unwrap_or_else(|| IndexCache {
        schema: CACHE_SCHEMA,
        acf: stamps.clone(),
        manifests: BTreeMap::new(),
    });

    let mut mappings: Vec<(String, Vec<ManifestFile>)> = Vec::new();
    let mut missing = 0usize;
    let mut broken = 0usize;
    let mut rebuilt = 0usize;
    let mut dirty = false;
    let mut used: Vec<String> = Vec::new();

    for (published_id, manifest_id) in &published {
        let Some(path) = manifest_path(depot_caches, manifest_id) else {
            missing += 1;
            continue;
        };
        used.push(manifest_id.clone());
        archive_manifest(&path, published_id, manifest_id);
        if let Some(files) = load_manifest_files(
            &path,
            manifest_id,
            &mut cache,
            &mut rebuilt,
            &mut broken,
            &mut dirty,
        ) {
            mappings.push((published_id.clone(), files));
        }
    }

    // A previous version of an installed item is still identifiable from
    // its archived manifest, which is the only record of that version's
    // bytes once Steam deletes the manifest on update.
    for (published_id, manifest_id, path) in archived_manifests() {
        if published.iter().any(|(id, _)| id == &published_id)
            && !published
                .iter()
                .any(|(id, current)| id == &published_id && current == &manifest_id)
        {
            used.push(manifest_id.clone());
            if let Some(files) = load_manifest_files(
                &path,
                &manifest_id,
                &mut cache,
                &mut rebuilt,
                &mut broken,
                &mut dirty,
            ) {
                mappings.push((published_id, files));
            }
        }
    }

    // Drop manifests the ACF no longer names, so the cache tracks the
    // installed set instead of growing with every update.
    let before = cache.manifests.len();
    cache.manifests.retain(|id, _| used.contains(id));
    dirty |= cache.manifests.len() != before;
    if dirty {
        if let Err(error) = save_cache(cache_path, &cache) {
            eprintln!(
                "Warning: could not write {}: {error}; the index will be rebuilt next run",
                cache_path.display()
            );
        }
    }

    let mut index = from_mappings(mappings);
    index.legacy = legacy;
    index.missing = missing;
    index.broken = broken;
    index.rebuilt = rebuilt;
    index.elapsed = started.elapsed();
    index
}

/// Build the in-memory index. Each content key keeps its items ordered so
/// the choice is deterministic across runs and cache rebuilds.
pub fn from_mappings(mappings: Vec<(String, Vec<ManifestFile>)>) -> WorkshopIndex {
    let mut file_counts: HashMap<&str, usize> = HashMap::new();
    for (id, files) in &mappings {
        file_counts.insert(id.as_str(), files.len());
    }

    let mut by_content: HashMap<([u8; 20], u64), Vec<IndexEntry>> = HashMap::new();
    let mut by_name: HashMap<String, Vec<IndexEntry>> = HashMap::new();
    let mut files = 0usize;
    let mut pbo_entries = 0usize;
    for (published_id, entries) in &mappings {
        for file in entries {
            let entry = IndexEntry {
                published_id: published_id.clone(),
                path: file.path.clone(),
                size: file.size,
            };
            if let Some(name) = pbo_basename(&file.path) {
                by_name.entry(name).or_default().push(entry.clone());
                pbo_entries += 1;
            }
            by_content
                .entry((file.sha_content, file.size))
                .or_default()
                .push(entry);
            files += 1;
        }
    }

    let order = |left: &IndexEntry, right: &IndexEntry| {
        let left_count = file_counts
            .get(left.published_id.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        let right_count = file_counts
            .get(right.published_id.as_str())
            .copied()
            .unwrap_or(usize::MAX);
        left_count
            .cmp(&right_count)
            .then_with(|| numeric_key(&left.published_id).cmp(&numeric_key(&right.published_id)))
            .then_with(|| left.path.cmp(&right.path))
    };
    for entries in by_content.values_mut() {
        entries.sort_by(&order);
        entries.dedup();
    }
    for entries in by_name.values_mut() {
        entries.sort_by(&order);
        entries.dedup();
    }

    let names = by_name.len();
    WorkshopIndex {
        by_content,
        by_name,
        manifests: mappings.len(),
        legacy: 0,
        missing: 0,
        broken: 0,
        files,
        pbo_entries,
        names,
        rebuilt: 0,
        elapsed: std::time::Duration::ZERO,
    }
}

/// The lowercased basename of a PBO path in a manifest. Only `.pbo` files
/// take part in name discovery; the content index still holds every file.
fn pbo_basename(path: &str) -> Option<String> {
    let base = path.rsplit(['\\', '/']).next()?;
    let lower = base.to_ascii_lowercase();
    lower.ends_with(".pbo").then_some(lower)
}

/// Order ids as numbers, so `999` sorts before `1000`.
fn numeric_key(id: &str) -> (bool, u64, &str) {
    match id.parse::<u64>() {
        Ok(value) => (true, value, id),
        Err(_) => (false, 0, id),
    }
}

/// SHA-1 of a file's contents and its size. `None` on read failure.
pub fn sha1_and_size(path: &Path) -> Option<([u8; 20], u64)> {
    use sha1::{Digest, Sha1};

    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha1::new();
    let mut buffer = vec![0u8; 1 << 16];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    let digest: [u8; 20] = hasher.finalize().into();
    Some((digest, size))
}

pub fn sha1_hex(sha: &[u8; 20]) -> String {
    sha.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Parse a 40-character lowercase or uppercase hex SHA-1.
pub fn sha1_from_hex(text: &str) -> Option<[u8; 20]> {
    if text.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for (index, pair) in text.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(pair).ok()?;
        out[index] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

/// Load the index for a command. `None` means the index is absent or has no
/// entries, and the caller keeps its existing behaviour.
pub fn load_index() -> Option<WorkshopIndex> {
    let index = build_index(&find_depot_caches(), &index_path());
    if index.is_empty() {
        return None;
    }
    if index.rebuilt > 0 {
        eprintln!(
            "Workshop index: {} manifests, {} PBO entries, {} names, {} files, {} legacy item(s), built in {} ms",
            index.manifests,
            index.pbo_entries,
            index.names,
            index.files,
            index.legacy,
            index.elapsed.as_millis()
        );
    }
    Some(index)
}

pub fn index_path() -> PathBuf {
    Path::new(".uksfta").join("workshop-index.json")
}

// --- Disk cache -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
struct FileStamp {
    len: u64,
    mtime_ns: u64,
}

impl FileStamp {
    fn read(path: &Path) -> Option<Self> {
        let metadata = fs::metadata(path).ok()?;
        let modified = metadata.modified().ok()?;
        let mtime_ns = modified.duration_since(UNIX_EPOCH).ok()?.as_nanos() as u64;
        Some(Self {
            len: metadata.len(),
            mtime_ns,
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IndexCache {
    schema: u32,
    acf: BTreeMap<String, FileStamp>,
    manifests: BTreeMap<String, CachedManifest>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedManifest {
    #[serde(flatten)]
    stamp: FileStamp,
    files: Vec<CachedFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedFile {
    p: String,
    s: u64,
    h: String,
}

impl CachedFile {
    fn from_file(file: &ManifestFile) -> Self {
        Self {
            p: file.path.clone(),
            s: file.size,
            h: sha1_hex(&file.sha_content),
        }
    }

    fn to_file(&self) -> Option<ManifestFile> {
        Some(ManifestFile {
            path: self.p.clone(),
            size: self.s,
            flags: 0,
            sha_content: sha1_from_hex(&self.h)?,
        })
    }
}

fn load_cache(path: &Path, acf: &BTreeMap<String, FileStamp>) -> Option<IndexCache> {
    let text = fs::read_to_string(path).ok()?;
    let cache: IndexCache = match serde_json::from_str(&text) {
        Ok(cache) => cache,
        Err(error) => {
            eprintln!(
                "Warning: {}: {error}; rebuilding the Workshop index",
                path.display()
            );
            return None;
        }
    };
    if cache.schema != CACHE_SCHEMA || &cache.acf != acf {
        return None;
    }
    Some(cache)
}

fn save_cache(path: &Path, cache: &IndexCache) -> Result<(), crate::error::UksftaError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(crate::error::UksftaError::Io)?;
    }
    let json = serde_json::to_vec(cache)
        .map_err(|error| crate::error::UksftaError::Parse(format!("workshop index: {error}")))?;
    write_atomic(path, &json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn varint(mut value: u64, out: &mut Vec<u8>) {
        loop {
            let byte = (value & 0x7f) as u8;
            value >>= 7;
            if value == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    fn bytes_field(field: u64, bytes: &[u8], out: &mut Vec<u8>) {
        varint((field << 3) | 2, out);
        varint(bytes.len() as u64, out);
        out.extend_from_slice(bytes);
    }

    fn varint_field(field: u64, value: u64, out: &mut Vec<u8>) {
        varint(field << 3, out);
        varint(value, out);
    }

    fn mapping(path: &str, size: u64, flags: u64, sha: [u8; 20]) -> Vec<u8> {
        let mut out = Vec::new();
        bytes_field(1, path.as_bytes(), &mut out);
        varint_field(2, size, &mut out);
        varint_field(3, flags, &mut out);
        bytes_field(4, &[0x11; 20], &mut out);
        bytes_field(5, &sha, &mut out);
        // A chunk message: field 6, itself a length-delimited message.
        let mut chunk = Vec::new();
        bytes_field(1, &[0x22; 20], &mut chunk);
        varint_field(4, size, &mut chunk);
        bytes_field(6, &chunk, &mut out);
        out
    }

    fn link_mapping(path: &str, target: &str) -> Vec<u8> {
        let mut out = Vec::new();
        bytes_field(1, path.as_bytes(), &mut out);
        varint_field(3, 0, &mut out);
        bytes_field(4, &[0x33; 20], &mut out);
        bytes_field(7, target.as_bytes(), &mut out);
        out
    }

    fn container(files: &[Vec<u8>], truncate_sig: bool) -> Vec<u8> {
        let mut payload = Vec::new();
        for file in files {
            bytes_field(1, file, &mut payload);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&MANIFEST_MAGIC.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&payload);
        out.extend_from_slice(&PAYLOAD_END_MAGIC.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&METADATA_END_MAGIC.to_le_bytes());
        if truncate_sig {
            return out;
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&SIGNATURE_END_MAGIC.to_le_bytes());
        out
    }

    fn sha(byte: u8) -> [u8; 20] {
        [byte; 20]
    }

    #[test]
    fn parses_file_mappings_and_skips_directories_and_links() {
        let data = container(
            &[
                mapping("addons", 0, FLAG_DIRECTORY, [0u8; 20]),
                mapping("addons\\a.pbo", 1234, 0, sha(0xaa)),
                mapping("Keys\\k.bikey", 42, 0, sha(0xbb)),
                link_mapping("link", "target"),
            ],
            false,
        );
        let files = parse_manifest(&data).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "addons\\a.pbo");
        assert_eq!(files[0].size, 1234);
        assert_eq!(files[0].sha_content, sha(0xaa));
        assert_eq!(files[1].path, "Keys\\k.bikey");
    }

    #[test]
    fn truncated_manifest_is_rejected() {
        let data = container(&[mapping("addons\\a.pbo", 1, 0, sha(0xaa))], true);
        assert!(matches!(
            parse_manifest(&data),
            Err(ManifestError::Truncated(_))
        ));
    }

    #[test]
    fn bad_magic_is_rejected() {
        let data = container(&[], false);
        let mut bad = data.clone();
        bad[0] ^= 0xff;
        assert!(matches!(
            parse_manifest(&bad),
            Err(ManifestError::Magic { .. })
        ));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut data = container(&[], false);
        data.push(0);
        assert!(matches!(
            parse_manifest(&data),
            Err(ManifestError::Trailing(1))
        ));
    }

    #[test]
    fn the_installed_copy_is_found_by_item_and_name() {
        let file = |path: &str, size: u64, byte: u8| ManifestFile {
            path: path.to_string(),
            size,
            flags: 0,
            sha_content: sha(byte),
        };
        let index = from_mappings(vec![
            (
                "3549882948".to_string(),
                vec![file("addons\\pdt_envg.pbo", 99, 0xaa)],
            ),
            (
                "686802825".to_string(),
                vec![file("Addons\\pdt_envg.pbo", 12, 0xbb)],
            ),
        ]);
        let own = index.installed_entry("3549882948", "pdt_envg.pbo").unwrap();
        assert_eq!(own.size, 99);
        // The name compare ignores case and the separator style.
        assert!(index
            .installed_entry("686802825", "PDT_ENVG.pbo")
            .is_some_and(|entry| entry.size == 12));
        assert!(index.installed_entry("3549882948", "absent.pbo").is_none());
        assert!(index.installed_entry("999", "pdt_envg.pbo").is_none());
    }

    #[test]
    fn an_excluded_item_stops_matching_its_own_files() {
        let file = |name: &str, byte: u8| ManifestFile {
            path: format!("addons\\{name}.pbo"),
            size: 10,
            flags: 0,
            sha_content: sha(byte),
        };
        let mut index = from_mappings(vec![
            ("3549882948".to_string(), vec![file("own", 0xaa)]),
            (
                "686802825".to_string(),
                vec![file("own", 0xaa), file("shared", 0xbb)],
            ),
        ]);
        assert_eq!(index.lookup(&sha(0xaa), 10).len(), 2);
        assert_eq!(index.files, 3);
        assert_eq!(index.manifests, 2);
        assert_eq!(index.pbo_entries, 3);

        index.exclude_item("3549882948");

        // The pack no longer confirms against itself, but a genuine other
        // publisher of the same bytes still does.
        let entries = index.lookup(&sha(0xaa), 10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].published_id, "686802825");
        assert_eq!(index.lookup(&sha(0xbb), 10).len(), 1);
        assert_eq!(index.files, 2);
        assert_eq!(index.manifests, 1);
        assert_eq!(index.pbo_entries, 2);
        assert_eq!(index.names, 2);

        // Excluding an item that is not in the index changes nothing.
        index.exclude_item("999");
        assert_eq!(index.files, 2);
        assert_eq!(index.manifests, 1);
    }

    #[test]
    fn lookup_hits_and_misses() {
        let index = from_mappings(vec![
            (
                "200".to_string(),
                vec![ManifestFile {
                    path: "addons\\a.pbo".to_string(),
                    size: 10,
                    flags: 0,
                    sha_content: sha(0xaa),
                }],
            ),
            (
                "100".to_string(),
                vec![ManifestFile {
                    path: "addons\\a.pbo".to_string(),
                    size: 10,
                    flags: 0,
                    sha_content: sha(0xaa),
                }],
            ),
        ]);
        // Both items hold the bytes; the tie is broken by numeric id.
        let entries = index.lookup(&sha(0xaa), 10);
        assert_eq!(entries.len(), 2);
        assert_eq!(index.pick(entries, None).unwrap().published_id, "100");
        assert_eq!(
            index.pick(entries, Some("200")).unwrap().published_id,
            "200"
        );
        assert!(index.lookup(&sha(0xaa), 11).is_empty());
        assert!(index.lookup(&sha(0xbb), 10).is_empty());
    }

    #[test]
    fn fewest_files_wins_the_tie() {
        let index = from_mappings(vec![
            (
                "111".to_string(),
                vec![
                    ManifestFile {
                        path: "addons\\pack.pbo".to_string(),
                        size: 5,
                        flags: 0,
                        sha_content: sha(0xcc),
                    },
                    ManifestFile {
                        path: "addons\\other.pbo".to_string(),
                        size: 5,
                        flags: 0,
                        sha_content: sha(0xdd),
                    },
                ],
            ),
            (
                "222".to_string(),
                vec![ManifestFile {
                    path: "addons\\pack.pbo".to_string(),
                    size: 5,
                    flags: 0,
                    sha_content: sha(0xcc),
                }],
            ),
        ]);
        let entries = index.lookup(&sha(0xcc), 5);
        assert_eq!(index.pick(entries, None).unwrap().published_id, "222");
    }

    fn scratch(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("uksfta-manifest-{name}-{unique}"));
        fs::create_dir_all(dir.join("depotcache")).unwrap();
        dir
    }

    fn write_acf(dir: &Path, items: &[(&str, &str)]) -> PathBuf {
        let mut text = String::from("\"AppWorkshop\"\n{\n\t\"WorkshopItemsInstalled\"\n\t{\n");
        for (id, manifest) in items {
            text.push_str(&format!(
                "\t\t\"{id}\"\n\t\t{{\n\t\t\t\"size\"\t\t\"1\"\n\t\t\t\"timeupdated\"\t\t\"1\"\n\t\t\t\"manifest\"\t\t\"{manifest}\"\n\t\t}}\n"
            ));
        }
        text.push_str("\t}\n}\n");
        let path = dir.join("appworkshop_107410.acf");
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn legacy_items_are_skipped_and_counted() {
        let dir = scratch("legacy");
        let acf = write_acf(&dir, &[("111", LEGACY_MANIFEST), ("222", "555")]);
        let cache = dir.join("depotcache");
        fs::write(
            cache.join("107410_555.manifest"),
            container(&[mapping("addons\\a.pbo", 7, 0, sha(0xee))], false),
        )
        .unwrap();

        let index = build_from(&[acf], &[cache], &dir.join("index.json"));
        assert_eq!(index.legacy, 1);
        assert_eq!(index.manifests, 1);
        assert_eq!(index.files, 1);
        assert_eq!(
            index.lookup(&sha(0xee), 7)[0].published_id,
            "222".to_string()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_manifest_is_ignored_with_a_warning() {
        let dir = scratch("corrupt");
        let acf = write_acf(&dir, &[("111", "555")]);
        let cache = dir.join("depotcache");
        fs::write(cache.join("107410_555.manifest"), b"\xd0\x17\xf6\x71\xff").unwrap();

        let index = build_from(&[acf], &[cache], &dir.join("index.json"));
        assert_eq!(index.broken, 1);
        assert!(index.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_is_reused_until_a_manifest_changes() {
        let dir = scratch("cache");
        let acf = write_acf(&dir, &[("111", "555")]);
        let cache = dir.join("depotcache");
        let manifest = cache.join("107410_555.manifest");
        fs::write(
            &manifest,
            container(&[mapping("addons\\a.pbo", 7, 0, sha(0x11))], false),
        )
        .unwrap();
        let index_path = dir.join("index.json");

        let first = build_from(
            std::slice::from_ref(&acf),
            std::slice::from_ref(&cache),
            &index_path,
        );
        assert_eq!(first.lookup(&sha(0x11), 7).len(), 1);
        assert!(index_path.is_file());

        // Same inputs: the cache stands and the mapping is unchanged.
        let second = build_from(
            std::slice::from_ref(&acf),
            std::slice::from_ref(&cache),
            &index_path,
        );
        assert_eq!(second.lookup(&sha(0x11), 7).len(), 1);

        // A changed manifest is re-read even when its length is the same.
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(
            &manifest,
            container(&[mapping("addons\\a.pbo", 7, 0, sha(0x22))], false),
        )
        .unwrap();
        let third = build_from(&[acf], &[cache], &index_path);
        assert!(third.lookup(&sha(0x11), 7).is_empty());
        assert_eq!(third.lookup(&sha(0x22), 7).len(), 1);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hex_round_trips() {
        let text = sha1_hex(&sha(0xab));
        assert_eq!(text, "ab".repeat(20));
        assert_eq!(sha1_from_hex(&text), Some(sha(0xab)));
        assert_eq!(sha1_from_hex("zz"), None);
    }

    fn file(path: &str, size: u64, byte: u8) -> ManifestFile {
        ManifestFile {
            path: path.to_string(),
            size,
            flags: 0,
            sha_content: sha(byte),
        }
    }

    #[test]
    fn name_candidates_narrow_by_size() {
        let index = from_mappings(vec![
            (
                "111".to_string(),
                vec![
                    file("addons\\Shared.pbo", 100, 0x01),
                    file("addons\\only.pbo", 50, 0x02),
                ],
            ),
            (
                "222".to_string(),
                vec![file("Addons\\shared.PBO", 200, 0x03)],
            ),
        ]);

        // The size keeps only the item that lists that size.
        let exact = index.candidates("SHARED.PBO", 100);
        assert!(exact.exact_size);
        assert_eq!(exact.ids(), vec!["111"]);

        // No item lists this size, so the name alone is returned, marked
        // inexact, ordered by fewest files in the item.
        let name_only = index.candidates("shared.pbo", 999);
        assert!(!name_only.exact_size);
        assert_eq!(name_only.ids(), vec!["222", "111"]);

        assert!(index.candidates("absent.pbo", 10).is_empty());
    }

    #[test]
    fn basename_handles_separators_and_case() {
        assert_eq!(
            pbo_basename("Addons\\ACE_Common.PBO"),
            Some("ace_common.pbo".to_string())
        );
        assert_eq!(
            pbo_basename("addons/ace_common.pbo"),
            Some("ace_common.pbo".to_string())
        );
        assert_eq!(pbo_basename("addons\\readme.txt"), None);
    }

    /// The real depot cache on this machine. Ignored by default.
    #[test]
    #[ignore]
    fn live_index_finds_a_known_pbo() {
        use crate::pbo_hashes::PboHashes;

        let caches = find_depot_caches();
        assert!(!caches.is_empty(), "no depot cache found");
        let index = build_index(&caches, &index_path());
        assert!(!index.is_empty(), "index is empty");

        // Take the first installed PBOs with a name candidate, hash them
        // through the production cache, and confirm each against its item.
        let mut paths = Vec::new();
        let mut expected = Vec::new();
        'outer: for cache in find_all_workshop_caches() {
            for entry in fs::read_dir(&cache).unwrap().flatten() {
                let id = entry.file_name().to_string_lossy().to_string();
                for pbo in crate::util::find_pbos(&entry.path()) {
                    if index.candidates_for(&pbo).is_empty() {
                        panic!("no name candidate for {}", pbo.display());
                    }
                    paths.push(pbo);
                    expected.push(id.clone());
                    if paths.len() == 3 {
                        break 'outer;
                    }
                }
            }
        }
        assert_eq!(paths.len(), 3, "too few installed PBOs");

        let cache_path = std::env::temp_dir().join("uksfta-live-pbo-hashes.json");
        let _ = fs::remove_file(&cache_path);
        let mut hashes = PboHashes::load(&cache_path);
        let digests = hashes.digests(&paths);
        assert_eq!(hashes.hashed, 3);
        hashes.finish();
        assert!(cache_path.is_file());

        for ((pbo, id), digest) in paths.iter().zip(&expected).zip(digests) {
            let (sha, size) = digest.expect("an installed PBO is readable");
            let entries = index.lookup(&sha, size);
            assert!(
                entries.iter().any(|entry| &entry.published_id == id),
                "{} matched the index but not item {}",
                pbo.display(),
                id
            );
            println!("confirmed {} -> item {}", pbo.display(), id);
        }
        fs::remove_file(&cache_path).unwrap();
    }
}
