use memchr::{memchr, memchr2, memmem};
use std::fs;
use std::path::{Path, PathBuf};

use crate::util::extract_quoted;

// --- mod.cpp / meta.cpp parsing ---

#[derive(Debug, Clone, Default)]
pub struct ModCpp {
    pub name: String,
    pub author: String,
    pub publishedid: Option<String>,
}

/// The trimmed left side of an `=` assignment, or None when the line has
/// no `=`. Exact-key matching stops `namespace` from matching `name`.
pub(crate) fn key_of(line: &str) -> Option<&str> {
    line.split_once('=').map(|(key, _)| key.trim())
}

fn parse_mod_cpp(path: &Path) -> ModCpp {
    let mut result = ModCpp::default();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = extract_quoted(line) {
                match key_of(line) {
                    Some("name") => result.name = val,
                    Some("author") => result.author = val,
                    _ => {}
                }
            }
        }
    }
    result
}

fn parse_meta_cpp(path: &Path) -> ModCpp {
    let mut result = ModCpp::default();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(val) = extract_quoted(line) {
                match key_of(line) {
                    Some("publishedid") => result.publishedid = Some(val),
                    Some("name") => result.name = val,
                    _ => {}
                }
            }
        }
    }
    result
}

pub fn get_mod_metadata(mod_dir: &Path) -> ModCpp {
    let mut meta = ModCpp::default();

    // mod.cpp has presentation metadata
    let mod_cpp = mod_dir.join("mod.cpp");
    if mod_cpp.exists() {
        meta = parse_mod_cpp(&mod_cpp);
    }

    // meta.cpp has Workshop ID
    let meta_cpp = mod_dir.join("meta.cpp");
    if meta_cpp.exists() {
        let workshop_meta = parse_meta_cpp(&meta_cpp);
        if let Some(id) = workshop_meta.publishedid {
            meta.publishedid = Some(id);
        }
        if meta.name.is_empty() && !workshop_meta.name.is_empty() {
            meta.name = workshop_meta.name;
        }
    }

    meta
}

/// Extract the addon prefix from a PBO header.
/// The prefix is the canonical virtual path (e.g. "z\ace\addons\grenades")
/// and is preserved when a pack re-packs a mod's PBO, so it identifies
/// the original addon even when the bytes differ.
pub fn pbo_prefix(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut file = fs::File::open(path).ok()?;
    let mut data = [0u8; 512];
    let n = file.read(&mut data).ok()?;
    let data = &data[..n];

    // Header entries are null-separated key\0value\0 pairs. Find "prefix".
    let key = b"prefix\x00";
    let idx = data.windows(key.len()).position(|w| w == key)?;
    let start = idx + key.len();
    let end = data[start..].iter().position(|&b| b == 0)? + start;
    let prefix = data[start..end].to_vec();
    String::from_utf8(prefix).ok()
}

/// One named entry in a PBO header. `offset` is the absolute file offset
/// of the entry's data, after the header and in header order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PboEntry {
    pub name: String,
    pub mime: u32,
    pub original_size: u32,
    pub data_size: u32,
    pub offset: u64,
}

/// Properties header, mapped from `Vers`.
const PBO_PROPERTIES_MIME: u32 = 0x5665_7273; // Vers
/// Compressed body; the raw bytes are returned for a later decoder.
pub(crate) const PBO_CPRS_MIME: u32 = 0x4370_7273; // Cprs
/// Encrypted body; never read.
const PBO_ENCR_MIME: u32 = 0x456e_6372; // Encr
const PBO_MAX_NAME: usize = 1024;
const PBO_MAX_HEADER_ENTRIES: usize = 100_000;
/// Total entry data read from one PBO by the text-source helpers.
const PBO_ENTRY_DATA_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
struct RawPboEntry {
    name: String,
    mime: u32,
    original_size: u32,
    data_size: u32,
}

fn read_cstring(reader: &mut impl std::io::Read, limit: usize) -> Option<String> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        match reader.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        if byte[0] == 0 {
            break;
        }
        if bytes.len() >= limit {
            return None;
        }
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).ok()
}

fn read_le_u32(reader: &mut impl std::io::Read) -> Option<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// Parse the header into named entries with data offsets. A malformed
/// header yields `None`: every name and data range is bounds-checked.
fn parse_pbo_entries(path: &Path) -> Option<Vec<PboEntry>> {
    use std::io::BufReader;

    // Names are read a byte at a time, so the file is buffered.
    let file = fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let mut file = BufReader::new(file);
    parse_pbo_header(&mut file, file_len).map(|(entries, _)| entries)
}

/// Parse the header from any seekable reader. Returns the entries and the
/// offset of the first entry's data, which is where the entry data area
/// starts.
fn parse_pbo_header(
    reader: &mut (impl std::io::Read + std::io::Seek),
    file_len: u64,
) -> Option<(Vec<PboEntry>, u64)> {
    // Properties header: empty name, mime `Vers`, four sizes. Its data is
    // inline and has no stored length: `key\0value\0...\0`.
    let name = read_cstring(reader, PBO_MAX_NAME)?;
    let mime = read_le_u32(reader)?;
    if !name.is_empty() || mime != PBO_PROPERTIES_MIME {
        return None;
    }
    for _ in 0..4 {
        read_le_u32(reader)?;
    }
    loop {
        let key = read_cstring(reader, PBO_MAX_NAME)?;
        if key.is_empty() {
            break;
        }
        read_cstring(reader, PBO_MAX_NAME)?;
    }

    let mut raw: Vec<RawPboEntry> = Vec::new();
    loop {
        if raw.len() >= PBO_MAX_HEADER_ENTRIES {
            return None;
        }
        let name = read_cstring(reader, PBO_MAX_NAME)?;
        let mime = read_le_u32(reader)?;
        let original_size = read_le_u32(reader)?;
        let reserved = read_le_u32(reader)?;
        let timestamp = read_le_u32(reader)?;
        let data_size = read_le_u32(reader)?;

        // Terminator: empty name plus twenty zero bytes.
        if name.is_empty()
            && mime == 0
            && original_size == 0
            && reserved == 0
            && timestamp == 0
            && data_size == 0
        {
            let mut offset = reader.stream_position().ok()?;
            let data_start = offset;
            let mut entries = Vec::new();
            for entry in raw {
                let end = offset.checked_add(u64::from(entry.data_size))?;
                if end > file_len {
                    return None;
                }
                if !entry.name.is_empty() {
                    entries.push(PboEntry {
                        name: entry.name,
                        mime: entry.mime,
                        original_size: entry.original_size,
                        data_size: entry.data_size,
                        offset,
                    });
                }
                offset = end;
            }
            return Some((entries, data_start));
        }

        raw.push(RawPboEntry {
            name,
            mime,
            original_size,
            data_size,
        });
    }
}

/// Every named entry in a PBO header. A malformed header yields an
/// empty list.
pub fn pbo_entries(path: &Path) -> Vec<PboEntry> {
    parse_pbo_entries(path).unwrap_or_default()
}

/// Entry data for the harvester: stored (`mime == 0`) entries only.
///
/// Compressed bodies are skipped until a decoder lands. The total is
/// capped at 64 MiB per PBO; the flag reports truncation.
pub fn pbo_stored_sources_capped(path: &Path) -> (Vec<(String, Vec<u8>)>, bool) {
    read_entry_data(path, &[0])
}

/// Stored and compressed entry data. `Encr` is skipped. A `Cprs` body
/// is returned raw under its entry name so a decoder can take it.
pub fn pbo_text_sources_capped(path: &Path) -> (Vec<(String, Vec<u8>)>, bool) {
    read_entry_data(path, &[0, PBO_CPRS_MIME])
}

/// Stored and compressed entry data, truncation flag dropped.
pub fn pbo_text_sources(path: &Path) -> Vec<(String, Vec<u8>)> {
    pbo_text_sources_capped(path).0
}

fn read_entry_data(path: &Path, mimes: &[u32]) -> (Vec<(String, Vec<u8>)>, bool) {
    let entries = pbo_entries(path);
    read_entry_data_of(path, &entries, mimes)
}

fn read_entry_data_of(
    path: &Path,
    entries: &[PboEntry],
    mimes: &[u32],
) -> (Vec<(String, Vec<u8>)>, bool) {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return (Vec::new(), false),
    };
    let mut sources = Vec::new();
    let mut total = 0u64;
    let mut truncated = false;

    for entry in entries {
        if entry.mime == PBO_ENCR_MIME || !mimes.contains(&entry.mime) {
            continue;
        }
        let size = u64::from(entry.data_size);
        if total + size > PBO_ENTRY_DATA_LIMIT {
            truncated = true;
            break;
        }
        if file.seek(SeekFrom::Start(entry.offset)).is_err() {
            truncated = true;
            break;
        }
        let mut data = Vec::new();
        if file.by_ref().take(size).read_to_end(&mut data).is_err() {
            truncated = true;
            break;
        }
        total += size;
        sources.push((entry.name.clone(), data));
    }
    (sources, truncated)
}

/// One PBO read once: header entries, the config scan window and the
/// stored entry data. The online path hands one read to both the
/// evidence harvester and the identity extractor.
#[derive(Debug, Clone, Default)]
pub struct PboContent {
    /// Source file.
    pub path: PathBuf,
    /// File name, as it appears in a report.
    pub name: String,
    /// The first [`PBO_SCAN_LIMIT`] bytes, where the header and the
    /// packed config sit.
    pub head: Vec<u8>,
    /// Stored entry data, in header order, capped at
    /// [`PBO_ENTRY_DATA_LIMIT`].
    pub sources: Vec<(String, Vec<u8>)>,
    /// Set when the entry data or the head was cut short.
    pub truncated: bool,
}

impl PboContent {
    /// Read one PBO. A file that cannot be read yields empty parts, which
    /// matches the empty result of each single-purpose helper.
    pub fn read(path: &Path) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let head = read_capped(path, PBO_SCAN_LIMIT).unwrap_or_default();
        let entries = pbo_entries(path);
        let (sources, truncated) = read_entry_data_of(path, &entries, &[0]);
        Self {
            path: path.to_path_buf(),
            name,
            head,
            sources,
            truncated,
        }
    }

    /// Config facts, read from the content already in memory.
    pub fn config_facts(&self) -> PboConfigFacts {
        config_facts_with(
            &self.head,
            &self.path,
            KeybindSources::Memory(&self.sources),
        )
    }
}

/// Config facts read from one capped pass over a PBO.
///
/// `required_addons` names dependencies and is corroboration only: it
/// never describes the addon's own identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PboConfigFacts {
    pub cfgmods_name: Option<String>,
    pub cfgmods_author: Option<String>,
    pub cfgpatches_class: Option<String>,
    /// Every `CfgPatches` class, for the dependency closure. Identity
    /// reads only `cfgpatches_class`.
    pub cfgpatches_classes: Vec<String>,
    pub cfgpatches_author: Option<String>,
    pub cfgpatches_url: Option<String>,
    pub display_names: Vec<String>,
    pub str_tokens: Vec<String>,
    pub required_addons: Vec<String>,
    /// Category strings from CBA keybind registrations. Modders usually
    /// put the mod title or the Workshop name here.
    pub cba_keybind_names: Vec<String>,
}

/// Bytes read from the start of a PBO. The header and the packed config
/// sit at the front.
const PBO_SCAN_LIMIT: u64 = 4_000_000;

/// First read of a PBO by the dependency closure. It covers the entry
/// table of every PBO in the pack except the largest map packs, which
/// grow it.
const PBO_CLOSURE_PROBE: u64 = 65_536;

/// Config region read after the entry table by the dependency closure.
///
/// Eight times the smallest window that leaves every closure fact
/// unchanged on the reference pack (8 KiB). The margin leaves room for an
/// unpacked `config.cpp`, which sits at the start of the data area.
const PBO_CLOSURE_CONFIG_WINDOW: u64 = 65_536;

/// Largest front buffer the closure holds. A header past it is read from
/// the file instead.
const PBO_CLOSURE_FRONT_LIMIT: u64 = PBO_SCAN_LIMIT;

/// Bytes hashed at each end of a PBO by [`pbo_fingerprint`].
const PBO_FINGERPRINT_WINDOW: u64 = 65_536;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Unknown,
    CfgMods,
    CfgPatches,
}

/// Extract every config fact the identity path needs, in one read.
///
/// Text configs are scanned as strings. A binarised (`\0raP`) config in
/// the same window is parsed and overwrites the scan: a real config is
/// stronger evidence than printable runs. A `Cprs`-compressed body
/// carries neither and yields the default facts.
pub fn pbo_config_facts(path: &Path) -> PboConfigFacts {
    let Some(head) = read_capped(path, PBO_SCAN_LIMIT) else {
        return PboConfigFacts::default();
    };
    config_facts_with(&head, path, KeybindSources::File(path))
}

/// The two config facts the dependency closure reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PboClosureFacts {
    pub cfgpatches_classes: Vec<String>,
    pub required_addons: Vec<String>,
}

/// `CfgPatches` classes and `requiredAddons` for one PBO.
///
/// The result matches [`pbo_config_facts`] for these two fields. The
/// identity-only scans are skipped, so no entry data is read beyond a
/// stored `config.bin`: models and textures are never touched. One open
/// and one bounded read of the front serve the entry table, the closure
/// markers and the stored config.
///
/// The two fields come from markers over the raw window, not from the
/// printable-run text, so no per-byte string is built for this path.
pub fn pbo_closure_facts(path: &Path) -> PboClosureFacts {
    let Some(mut front) = PboFront::open(path, PBO_CLOSURE_PROBE, PBO_CLOSURE_CONFIG_WINDOW) else {
        return PboClosureFacts::default();
    };
    let mut facts = closure_scan(&front.buf);
    if let Some(rap) = front.rap_facts() {
        merge_rap_facts(&mut facts, rap);
    }
    PboClosureFacts {
        cfgpatches_classes: facts.cfgpatches_classes,
        required_addons: facts.required_addons,
    }
}

/// The needle that starts a class declaration.
const CLASS_MARKER: &[u8] = b"class ";
/// The needle that starts a dependency list.
const REQUIRED_ADDONS_MARKER: &[u8] = b"requiredAddons";
/// The key [`config_list`] is given for a dependency list.
const REQUIRED_ADDONS_KEY: &str = "requiredAddons[]";

/// Whether the window can carry a closure fact at all.
///
/// A class only enters the scan through a `class ` statement, and an
/// addon only through a `requiredAddons[]` key. Both are printable, so a
/// byte search over the raw window is exact, and a binarised pack can
/// skip the marker walk.
fn has_closure_text(data: &[u8]) -> bool {
    memmem::find(data, CLASS_MARKER).is_some()
        || memmem::find(data, REQUIRED_ADDONS_KEY.as_bytes()).is_some()
}

/// The front of one PBO, read once.
///
/// The closure needs three things that all live at or near the front: the
/// entry table, the printable runs a text config sits in, and the stored
/// `config.bin` body. One open and one bounded read serve all three, and
/// the buffer grows only when the entry table runs past it.
struct PboFront {
    file: fs::File,
    len: u64,
    buf: Vec<u8>,
    entries: Option<Vec<PboEntry>>,
}

impl PboFront {
    fn open(path: &Path, probe: u64, config_window: u64) -> Option<Self> {
        let file = fs::File::open(path).ok()?;
        let len = file.metadata().ok()?.len();
        let mut front = Self {
            file,
            len,
            buf: Vec::new(),
            entries: None,
        };
        front.extend_to(probe)?;

        // The entry table ends the header, and the config region follows
        // it. Grow past the probe until the table parses.
        let limit = PBO_CLOSURE_FRONT_LIMIT.min(len);
        loop {
            let parsed = parse_pbo_header(&mut std::io::Cursor::new(&front.buf[..]), len);
            match parsed {
                Some((entries, data_start)) => {
                    // An unpacked config is an entry, so the window must
                    // reach past its body to be scanned at all.
                    let text_end = entries
                        .iter()
                        .find(|entry| entry.mime == 0 && is_text_config_entry(&entry.name))
                        .map(|entry| entry.offset.saturating_add(u64::from(entry.data_size)))
                        .unwrap_or(0);
                    let target = data_start
                        .saturating_add(config_window)
                        .max(text_end)
                        .min(PBO_CLOSURE_FRONT_LIMIT);
                    front.entries = Some(entries);
                    front.extend_to(target)?;
                    return Some(front);
                }
                None if (front.buf.len() as u64) < limit => {
                    let target = (front.buf.len() as u64).saturating_mul(2).min(limit);
                    front.extend_to(target)?;
                }
                None => return Some(front),
            }
        }
    }

    /// Read up to `target` bytes of the file into the buffer, appending.
    fn extend_to(&mut self, target: u64) -> Option<()> {
        use std::io::{Read, Seek, SeekFrom};

        let target = target.min(self.len) as usize;
        if self.buf.len() >= target {
            return Some(());
        }
        self.file
            .seek(SeekFrom::Start(self.buf.len() as u64))
            .ok()?;
        let want = (target - self.buf.len()) as u64;
        self.buf.reserve_exact(want as usize);
        self.file
            .by_ref()
            .take(want)
            .read_to_end(&mut self.buf)
            .ok()?;
        Some(())
    }

    /// The entry table. A header past the memory limit falls back to the
    /// reader that reads the whole file.
    fn entries(&mut self) -> &[PboEntry] {
        if self.entries.is_none() {
            let mut reader = std::io::BufReader::new(&self.file);
            let parsed = {
                use std::io::{Seek, SeekFrom};
                reader
                    .seek(SeekFrom::Start(0))
                    .ok()
                    .and_then(|_| parse_pbo_header(&mut reader, self.len))
                    .map(|(entries, _)| entries)
            };
            self.entries = Some(parsed.unwrap_or_default());
        }
        self.entries.as_deref().unwrap_or_default()
    }

    /// Facts from a stored `config.bin`, then from a `\0raP` blob in the
    /// window. Same order as the whole-file reader.
    fn rap_facts(&mut self) -> Option<RapFacts> {
        self.rap_from_config_entries()
            .or_else(|| parse_rap_config(&self.buf))
    }

    fn rap_from_config_entries(&mut self) -> Option<RapFacts> {
        use std::io::{Read, Seek, SeekFrom};

        let entries = self.entries().to_vec();
        rap_from_config_entries_with(&entries, |entry| {
            let end = entry.offset.checked_add(u64::from(entry.data_size));
            if let Some(end) = end.filter(|end| *end <= self.buf.len() as u64) {
                return Ok(Some(self.buf[entry.offset as usize..end as usize].to_vec()));
            }
            let mut data = Vec::new();
            self.file
                .seek(SeekFrom::Start(entry.offset))
                .map_err(|_| ())?;
            match self
                .file
                .by_ref()
                .take(u64::from(entry.data_size))
                .read_to_end(&mut data)
            {
                Ok(_) => Ok(Some(data)),
                Err(_) => Ok(None),
            }
        })
    }
}

/// The two closure fields from one pass over a PBO front.
///
/// [`has_closure_text`] is the pre-filter: a PBO front with neither
/// marker cannot declare a class or a dependency, so the rest of the
/// scan is skipped.
pub(crate) fn closure_scan(head: &[u8]) -> PboConfigFacts {
    if !has_closure_text(head) {
        return PboConfigFacts::default();
    }
    PboConfigFacts {
        cfgpatches_classes: config_classes(head),
        required_addons: config_required_addons(head),
        ..PboConfigFacts::default()
    }
}

/// Positions of the `{` and `}` bytes, in order.
struct Braces<'a> {
    data: &'a [u8],
    from: usize,
}

impl<'a> Braces<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, from: 0 }
    }
}

impl Iterator for Braces<'_> {
    type Item = (usize, u8);

    fn next(&mut self) -> Option<Self::Item> {
        let index = memchr2(b'{', b'}', self.data.get(self.from..)?)?;
        let position = self.from + index;
        self.from = position + 1;
        Some((position, self.data[position]))
    }
}

/// Every `CfgPatches` class the config declares, in declaration order and
/// deduplicated case-insensitively with the first spelling kept.
///
/// The rules are the ones [`scan_sections`] applies: a class statement one
/// level inside a `CfgPatches` block names a class the PBO provides. Only
/// the class markers and the braces matter for this, so the walk merges
/// the two ordered position streams and skips the rest of the window.
fn config_classes(head: &[u8]) -> Vec<String> {
    let mut classes: Vec<String> = Vec::new();
    let mut markers = memmem::find_iter(head, CLASS_MARKER);
    let Some(mut marker) = markers.next() else {
        return classes;
    };
    let mut braces = Braces::new(head);
    let mut brace_event = braces.next();
    let mut section = Section::Unknown;
    let mut section_depth = 0usize;
    let mut pending_section = Section::Unknown;
    let mut depth = 0usize;

    loop {
        let brace_first = match brace_event {
            Some((position, _)) => position < marker,
            None => false,
        };
        if brace_first {
            if let Some((_, byte)) = brace_event {
                if byte == b'{' {
                    depth += 1;
                    if pending_section != Section::Unknown {
                        section = std::mem::replace(&mut pending_section, Section::Unknown);
                        section_depth = depth - 1;
                    }
                } else {
                    depth = depth.saturating_sub(1);
                    if section != Section::Unknown && depth <= section_depth {
                        section = Section::Unknown;
                    }
                }
            }
            brace_event = braces.next();
            continue;
        }

        if let Some(name) = class_name_at(head, marker) {
            if name.eq_ignore_ascii_case("cfgmods") {
                pending_section = Section::CfgMods;
            } else if name.eq_ignore_ascii_case("cfgpatches") {
                pending_section = Section::CfgPatches;
            } else if section == Section::CfgPatches
                && depth == section_depth + 1
                && !classes.iter().any(|seen| seen.eq_ignore_ascii_case(name))
            {
                classes.push(name.to_string());
            }
        }

        match markers.next() {
            Some(position) => marker = position,
            None => break,
        }
    }

    classes
}

/// The name in one `class` declaration at `marker`, or None when the
/// marker does not start a statement.
///
/// This is [`class_name`] over the raw window: a statement starts at the
/// window, at a brace, or after a `;`, and only spaces may sit between
/// that boundary and the marker.
fn class_name_at(data: &[u8], marker: usize) -> Option<&str> {
    let mut statement = marker;
    while statement > 0 && data[statement - 1] == b' ' {
        statement -= 1;
    }
    if let Some(&previous) = statement.checked_sub(1).and_then(|at| data.get(at)) {
        if is_run_byte(previous) && !matches!(previous, b'{' | b'}' | b';') {
            return None;
        }
    }

    let mut start = marker + CLASS_MARKER.len();
    while start < data.len() && data[start] == b' ' {
        start += 1;
    }
    let mut end = start;
    while end < data.len() {
        if !is_run_byte(data[end]) || matches!(data[end], b' ' | b':' | b'{' | b';') {
            break;
        }
        end += 1;
    }
    let name = std::str::from_utf8(data.get(start..end)?)
        .ok()?
        .trim_matches('"');
    (!name.is_empty()).then_some(name)
}

/// Every `requiredAddons[]` entry the config declares, in declaration
/// order and deduplicated.
///
/// The marker gives the start of a list. The window runs to the brace that
/// closes it, which is the same brace [`config_list`] stops at, and only
/// that window is projected to text, so the parse rules stay identical.
fn config_required_addons(head: &[u8]) -> Vec<String> {
    let mut addons: Vec<String> = Vec::new();
    let mut from = 0usize;
    while let Some(hit) = head
        .get(from..)
        .and_then(|rest| memmem::find(rest, REQUIRED_ADDONS_MARKER))
    {
        let at = from + hit;
        let Some(close) = head.get(at..).and_then(|rest| memchr(b'}', rest)) else {
            break;
        };
        let close = at + close;
        let window = readable_strings(&head[at..=close]);
        let (found, _) = config_list_step(&window, 0, REQUIRED_ADDONS_KEY);
        match found {
            Some(entries) => {
                for addon in entries {
                    if !addons.contains(&addon) {
                        addons.push(addon);
                    }
                }
                from = close + 1;
            }
            None => from = at + REQUIRED_ADDONS_KEY.len(),
        }
    }
    addons
}

fn config_facts_with(head: &[u8], path: &Path, keybinds: KeybindSources<'_>) -> PboConfigFacts {
    let text = readable_strings(head);
    let rap = rap_from_config_entries(path).or_else(|| parse_rap_config(head));
    let mut facts = base_facts(&text, rap);
    facts.str_tokens = str_tokens(&text);
    facts.cba_keybind_names = keybind_names(&text, keybinds);
    facts
}

/// Everything one config carries, apart from the identity-only scans.
fn base_facts(text: &str, rap: Option<RapFacts>) -> PboConfigFacts {
    let mut facts = PboConfigFacts {
        required_addons: required_addons(text),
        ..PboConfigFacts::default()
    };
    scan_sections(text, &mut facts);

    if let Some(rap) = rap {
        merge_rap_facts(&mut facts, rap);
    }

    facts
}

/// Walk the config text and record the class declarations and the
/// assignments the identity path reads.
fn scan_sections(text: &str, facts: &mut PboConfigFacts) {
    // `section` is the CfgMods or CfgPatches block the scan is inside;
    // `section_depth` is the brace depth at which that block opened.
    let mut section = Section::Unknown;
    let mut section_depth = 0usize;
    let mut pending_section = Section::Unknown;
    let mut depth = 0usize;

    for statement in statements(text) {
        match statement {
            "{" => {
                depth += 1;
                if pending_section == Section::Unknown {
                    continue;
                }
                section = std::mem::replace(&mut pending_section, Section::Unknown);
                section_depth = depth - 1;
                continue;
            }
            "}" => {
                depth = depth.saturating_sub(1);
                if section != Section::Unknown && depth <= section_depth {
                    section = Section::Unknown;
                }
                continue;
            }
            _ => {}
        }

        if let Some(name) = class_name(statement) {
            match name.to_ascii_lowercase().as_str() {
                "cfgmods" => pending_section = Section::CfgMods,
                "cfgpatches" => pending_section = Section::CfgPatches,
                _ if section == Section::CfgPatches && depth == section_depth + 1 => {
                    if facts.cfgpatches_class.is_none() {
                        facts.cfgpatches_class = Some(name.to_string());
                    }
                    if !facts
                        .cfgpatches_classes
                        .iter()
                        .any(|seen| seen.eq_ignore_ascii_case(name))
                    {
                        facts.cfgpatches_classes.push(name.to_string());
                    }
                }
                _ => {}
            }
            continue;
        }

        let Some((key, value)) = statement.split_once('=') else {
            continue;
        };
        let value = extract_cfg_value(value);
        match key.trim() {
            "name" if section == Section::CfgMods => set_first(&mut facts.cfgmods_name, value),
            "author" => match section {
                Section::CfgMods => set_author(&mut facts.cfgmods_author, value),
                Section::Unknown | Section::CfgPatches => {
                    set_author(&mut facts.cfgpatches_author, value);
                }
            },
            "displayName" => {
                if let Some(name) = value.filter(|name| !name.is_empty()) {
                    facts.display_names.push(name);
                }
            }
            "url" if section != Section::CfgMods => {
                if let Some(url) = value.filter(|url| url.starts_with("http")) {
                    set_first(&mut facts.cfgpatches_url, Some(url));
                }
            }
            _ => {}
        }
    }
}

/// Addon classes the PBO provides: every `CfgPatches` class in its config.
///
/// Deduplicated case-insensitively, first spelling kept, sorted. A PBO
/// with no readable config yields an empty list.
pub fn pbo_provided_classes(path: &Path) -> Vec<String> {
    let mut classes: Vec<String> = Vec::new();
    for class in pbo_config_facts(path).cfgpatches_classes {
        if !classes.iter().any(|seen| seen.eq_ignore_ascii_case(&class)) {
            classes.push(class);
        }
    }
    classes.sort_by_key(|class| class.to_ascii_lowercase());
    classes
}

/// Content fingerprint: sha256 over the file length, the first 64 KiB
/// and the last 64 KiB. Two copies of the same content share a
/// fingerprint even when their paths differ.
pub fn pbo_fingerprint(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};

    let len = fs::metadata(path).ok()?.len();
    let head = read_capped(path, PBO_FINGERPRINT_WINDOW)?;

    let mut tail = Vec::new();
    let mut file = fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(len.saturating_sub(PBO_FINGERPRINT_WINDOW)))
        .ok()?;
    file.take(PBO_FINGERPRINT_WINDOW)
        .read_to_end(&mut tail)
        .ok()?;

    let mut hasher = Sha256::new();
    hasher.update(len.to_le_bytes());
    hasher.update(&head);
    hasher.update(&tail);
    Some(
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// Entry names that carry authored text rather than models or textures.
fn is_authored_text(name: &str) -> bool {
    const EXTENSIONS: &[&str] = &[
        "cpp", "hpp", "sqf", "ext", "cfg", "h", "txt", "xml", "json", "md", "inc", "rpt",
    ];
    name.rsplit_once('.')
        .map(|(_, extension)| EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Where the keybind entry data comes from.
enum KeybindSources<'a> {
    /// Entry data the caller already holds.
    Memory(&'a [(String, Vec<u8>)]),
    /// A PBO whose authored-text entries are read one at a time, and only
    /// when the head carries no keybind.
    File(&'a Path),
}

/// Categories from CBA keybind registrations: first the 4 MiB window,
/// then the stored authored-text entries, because a mod's keybind code is
/// often near the end of a large PBO.
///
/// Entry names are filtered before any data is read, so a model or a
/// texture is never read. Every authored-text entry is still scanned,
/// because a PBO may register categories in more than one file, and the
/// union is the same as a scan of all stored entries.
fn keybind_names(head: &str, sources: KeybindSources<'_>) -> Vec<String> {
    let mut names = cba_keybind_names(head);
    if names.is_empty() {
        names = match sources {
            KeybindSources::Memory(sources) => from_held_sources(sources),
            KeybindSources::File(path) => from_entry_data(path),
        };
    }
    names.sort();
    names.dedup();
    names
}

fn from_held_sources(sources: &[(String, Vec<u8>)]) -> Vec<String> {
    let mut names = Vec::new();
    for (name, bytes) in sources {
        if is_authored_text(name) {
            names.extend(cba_keybind_names(&readable_strings(bytes)));
        }
    }
    names
}

/// Read the authored-text entries in header order. The budget covers
/// every stored entry, so the entries read match [`read_entry_data_of`]
/// with the stored mime.
fn from_entry_data(path: &Path) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let mut total = 0u64;
    for entry in pbo_entries(path) {
        if entry.mime != 0 {
            continue;
        }
        let size = u64::from(entry.data_size);
        if total + size > PBO_ENTRY_DATA_LIMIT {
            break;
        }
        total += size;
        if !is_authored_text(&entry.name) {
            continue;
        }
        if file.seek(SeekFrom::Start(entry.offset)).is_err() {
            break;
        }
        let mut data = Vec::new();
        if file.by_ref().take(size).read_to_end(&mut data).is_err() {
            break;
        }
        names.extend(cba_keybind_names(&readable_strings(&data)));
    }
    names
}

/// The first quoted string of a `CBA_fnc_addKeybind` array. The marker
/// closes the array, so the category is on the same printable line.
pub(crate) fn cba_keybind_names(text: &str) -> Vec<String> {
    const MARKER: &str = "CBA_fnc_addKeybind";
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(position) = rest.find(MARKER) {
        let before = &rest[..position];
        // The array can contain newlines and semicolons of its own, so the
        // anchor is its opening `["`, the first string element. Nested
        // string arrays are not used in keybind registrations.
        if let Some(open) = before.rfind("[\"") {
            let after = &before[open + 2..];
            if let Some(close) = after.find('"') {
                let candidate = after[..close].trim();
                if is_mod_name_shape(candidate) {
                    names.push(candidate.to_string());
                }
            }
        }
        rest = &rest[position + MARKER.len()..];
    }
    names
}

/// A plausible mod title rather than an identifier, a path or a code
/// token. Deliberately structural: no mod names.
fn is_mod_name_shape(text: &str) -> bool {
    let length = text.chars().count();
    if !(3..=80).contains(&length) {
        return false;
    }
    if text.starts_with('$') || text.starts_with('#') {
        return false;
    }
    if text.contains('\\') || text.contains('/') || text.contains('%') {
        return false;
    }
    if text.chars().all(|character| character.is_ascii_digit()) {
        return false;
    }
    if text.contains('_') && !text.contains(' ') {
        return false;
    }
    text.chars().any(|character| character.is_alphabetic())
}

fn read_capped(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read;

    let file = fs::File::open(path).ok()?;
    let mut data = Vec::new();
    file.take(limit).read_to_end(&mut data).ok()?;
    Some(data)
}

/// Shortest printable run kept. Shorter runs are noise.
const RUN_MIN: usize = 4;
/// One per byte, for the eight-byte printable test.
const BYTE_ONES: u64 = 0x0101_0101_0101_0101;
/// The high bit of every byte lane.
const BYTE_HIGH: u64 = 0x8080_8080_8080_8080;

/// True for a byte a printable run keeps. Braces are handled before this
/// test, so they never reach it inside a chunk.
const fn is_run_byte(byte: u8) -> bool {
    byte.wrapping_sub(0x20) < 0x5f
}

/// True when all eight bytes are printable. A false "no" only sends the
/// caller back to one byte at a time, so the carry of the second test is
/// allowed to be conservative.
fn all_run_bytes(chunk: &[u8]) -> bool {
    let mut word = [0u8; 8];
    word.copy_from_slice(chunk);
    let value = u64::from_le_bytes(word);
    let below = value.wrapping_sub(BYTE_ONES * 0x20) & !value & BYTE_HIGH;
    let above = value.wrapping_add(BYTE_ONES) | value;
    (below | above) & BYTE_HIGH == 0
}

/// The next byte at or after `from` that a run does not keep.
fn next_non_run(data: &[u8], mut from: usize) -> usize {
    while from + 8 <= data.len() {
        if !all_run_bytes(&data[from..from + 8]) {
            break;
        }
        from += 8;
    }
    while from < data.len() && is_run_byte(data[from]) {
        from += 1;
    }
    from
}

/// Printable runs of at least four bytes, one per line, so the field
/// scans can work line by line. Braces get their own line: they carry
/// the nesting the scan needs.
pub(crate) fn readable_strings(data: &[u8]) -> String {
    // Braces split the input, so one search covers both the run scan and
    // the brace lines; the runs are written in place and converted once.
    let mut text: Vec<u8> = Vec::with_capacity(data.len() + data.len() / 8);
    let mut rest = data;
    while let Some(index) = memchr2(b'{', b'}', rest) {
        push_runs(&mut text, &rest[..index]);
        text.push(rest[index]);
        text.push(b'\n');
        rest = &rest[index + 1..];
    }
    push_runs(&mut text, rest);
    // Only printable ASCII, braces and newlines were written.
    String::from_utf8(text).expect("readable strings are ASCII")
}

fn push_runs(text: &mut Vec<u8>, chunk: &[u8]) {
    let mut start = 0usize;
    loop {
        let end = next_non_run(chunk, start);
        if end - start >= RUN_MIN {
            text.extend_from_slice(&chunk[start..end]);
            text.push(b'\n');
        }
        if end >= chunk.len() {
            return;
        }
        start = end + 1;
    }
}

/// Split config text into class declarations, braces, statements and
/// terminators, so a one-line config parses like a formatted one.
fn statements(text: &str) -> impl Iterator<Item = &str> {
    text.split([';', '\n'])
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
}

fn class_name(statement: &str) -> Option<&str> {
    let rest = statement.strip_prefix("class ")?.trim_start();
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ':' || c == '{')
        .unwrap_or(rest.len());
    let name = rest[..end].trim().trim_matches('"');
    (!name.is_empty()).then_some(name)
}

fn set_first(slot: &mut Option<String>, value: Option<String>) {
    if slot.is_none() {
        *slot = value;
    }
}

/// Record an author. A bracketed author tag (`[ADE]`) is the mod's
/// identity, so it replaces a plain name captured earlier: the plain
/// name is usually the person who packed the PBO.
fn set_author(slot: &mut Option<String>, value: Option<String>) {
    let Some(value) = value else {
        return;
    };
    let bracketed = value.starts_with('[');
    match slot {
        None => *slot = Some(value),
        Some(existing) if bracketed && !existing.starts_with('[') => *slot = Some(value),
        Some(_) => {}
    }
}

/// Every entry of a `key[] = { ... }` list, in declaration order. The
/// scan tolerates the array wrapping across lines, and strips quotes
/// and whitespace from each entry.
pub(crate) fn config_list(text: &str, key: &str) -> Vec<String> {
    let mut addons: Vec<String> = Vec::new();
    let mut from = 0usize;
    while let Some(hit) = text.get(from..).and_then(|rest| rest.find(key)) {
        let (found, resume) = config_list_step(text, from + hit, key);
        if let Some(entries) = found {
            for addon in entries {
                if !addons.contains(&addon) {
                    addons.push(addon);
                }
            }
        }
        from = resume;
    }
    addons
}

/// One `key[] = { ... }` list that starts at `index`, and the position to
/// resume the search from: past the closing brace when a list was found,
/// past the key when the key does not start one.
fn config_list_step(text: &str, index: usize, key: &str) -> (Option<Vec<String>>, usize) {
    let after_key = index + key.len();
    let not_a_list = (None, after_key);
    let Some(rest) = text.get(after_key..) else {
        return not_a_list;
    };
    let Some(rest) = rest.trim_start().strip_prefix('=') else {
        return not_a_list;
    };
    let Some(rest) = rest.trim_start().strip_prefix('{') else {
        return not_a_list;
    };
    let Some((body, after_close)) = rest.split_once('}') else {
        return not_a_list;
    };
    let mut entries: Vec<String> = Vec::new();
    for entry in body.split(',') {
        let addon: String = entry
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '"')
            .collect();
        if !addon.is_empty() && !entries.contains(&addon) {
            entries.push(addon);
        }
    }
    (Some(entries), text.len() - after_close.len())
}

/// Every `requiredAddons[]` entry from one config text.
fn required_addons(text: &str) -> Vec<String> {
    config_list(text, "requiredAddons[]")
}

/// Every distinct `$STR_` token, prefix included.
fn str_tokens(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("$STR_") {
        rest = &rest[index..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
            .unwrap_or(rest.len());
        let token = &rest[..end];
        if !tokens.iter().any(|seen| seen == token) {
            tokens.push(token.to_string());
        }
        rest = &rest[end..];
    }
    tokens
}

/// Extract a quoted or bare value from a config line after `=`.
fn extract_cfg_value(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(s) = s.strip_prefix('"') {
        if let Some(end) = s.find('"') {
            return Some(s[..end].to_string());
        }
    } else {
        // Bare value: take until whitespace or semicolon
        let end = s
            .find(|c: char| c.is_whitespace() || c == ';')
            .unwrap_or(s.len());
        if end > 0 {
            return Some(s[..end].to_string());
        }
    }
    None
}

// --- binarised config (`raP`) ---

/// Magic that opens a binarised config blob.
const RAP_MAGIC: &[u8] = b"\0raP";
/// Bytes before the config body: the magic and three `u32` words.
const RAP_HEADER_LEN: usize = 16;
/// Class bodies nested deeper than this are not followed.
const RAP_MAX_DEPTH: usize = 129;
/// Class entries walked in one blob.
const RAP_MAX_CLASSES: usize = 100_000;
/// Entries or array elements read from one body.
const RAP_MAX_CHILDREN: usize = 100_000;
/// Blobs tried in one scan window.
const RAP_MAX_BLOBS: usize = 8;
/// Bytes read from one stored config entry.
const PBO_CONFIG_ENTRY_LIMIT: u64 = 16 * 1024 * 1024;
/// Stored config entries tried per PBO.
const PBO_MAX_CONFIG_ENTRIES: usize = 8;

/// Facts recovered from a binarised config.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RapFacts {
    cfgpatches_class: Option<String>,
    cfgpatches_classes: Vec<String>,
    cfgpatches_author: Option<String>,
    cfgpatches_url: Option<String>,
    required_addons: Vec<String>,
    cfgmods_name: Option<String>,
    cfgmods_author: Option<String>,
    cfgmods_url: Option<String>,
}

impl RapFacts {
    fn is_empty(&self) -> bool {
        self.cfgpatches_class.is_none()
            && self.cfgpatches_classes.is_empty()
            && self.cfgpatches_author.is_none()
            && self.cfgpatches_url.is_none()
            && self.required_addons.is_empty()
            && self.cfgmods_name.is_none()
            && self.cfgmods_author.is_none()
            && self.cfgmods_url.is_none()
    }
}

/// A cursor over one byte range. Every read is bounds-checked and returns
/// `None` at the end, so a truncated blob cannot panic.
struct RapReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> RapReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn byte(&mut self) -> Option<u8> {
        let byte = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(byte)
    }

    fn u32(&mut self) -> Option<u32> {
        let end = self.pos.checked_add(4)?;
        let word = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(u32::from_le_bytes(word.try_into().ok()?))
    }

    fn skip(&mut self, count: usize) -> Option<()> {
        self.pos = self.pos.checked_add(count)?;
        (self.pos <= self.bytes.len()).then_some(())
    }

    /// A null-terminated string. The outer `None` is a truncated read, the
    /// inner one a run that is not UTF-8: the cursor advances either way.
    fn cstring(&mut self) -> Option<Option<&'a str>> {
        let rest = self.bytes.get(self.pos..)?;
        let end = rest.iter().position(|&byte| byte == 0)?;
        self.pos += end + 1;
        Some(std::str::from_utf8(&rest[..end]).ok())
    }

    /// Compact integer: seven payload bits per byte, least significant
    /// group first, high bit set on every byte but the last.
    fn cint(&mut self) -> Option<usize> {
        let mut value = 0u64;
        for group in 0..5 {
            let byte = self.byte()?;
            value |= u64::from(byte & 0x7f) << (7 * group);
            if byte & 0x80 == 0 {
                return usize::try_from(value).ok();
            }
        }
        None
    }
}

/// One binarised config being walked. `blob` is the whole `\0raP` range:
/// a class offset names a position in it. `open` holds the offsets of the
/// bodies on the current path, so a class that points back at an ancestor
/// is skipped instead of followed for ever.
struct RapParser<'a> {
    blob: &'a [u8],
    facts: RapFacts,
    classes: usize,
    open: Vec<usize>,
}

impl<'a> RapParser<'a> {
    fn walk(
        &mut self,
        mut reader: RapReader<'a>,
        path: &mut Vec<&'a str>,
        depth: usize,
    ) -> Option<()> {
        reader.cstring()?; // parent class name
        let count = reader.cint()?;
        if count > RAP_MAX_CHILDREN {
            return None;
        }

        for _ in 0..count {
            if self.classes > RAP_MAX_CLASSES {
                return Some(());
            }
            match reader.byte()? {
                0 => {
                    let name = reader.cstring()?;
                    let offset = reader.u32()? as usize;
                    self.classes += 1;
                    let Some(name) = name else { continue };
                    self.note_class(path, name);
                    self.descend(offset, name, path, depth)?;
                }
                1 => {
                    let subtype = reader.byte()?;
                    let name = reader.cstring()?;
                    let value = match subtype {
                        0 => reader.cstring()?,
                        1 | 2 => {
                            reader.skip(4)?;
                            None
                        }
                        // An unknown payload has no known width: skip the
                        // property and keep reading the sibling entries.
                        _ => None,
                    };
                    if let Some(name) = name {
                        self.note_value(path, name, value);
                    }
                }
                2 => {
                    let name = reader.cstring()?;
                    let values = self.array(&mut reader, depth)?;
                    if let Some(name) = name {
                        self.note_array(path, name, &values);
                    }
                }
                5 => {
                    reader.skip(4)?; // property uid
                    let name = reader.cstring()?;
                    let values = self.array(&mut reader, depth)?;
                    if let Some(name) = name {
                        self.note_array(path, name, &values);
                    }
                }
                3 | 4 => {
                    reader.cstring()?; // extern class, deleted class
                }
                _ => return None,
            }
        }
        Some(())
    }

    /// Follow one class body. The outer cursor is untouched, so the walk
    /// resumes after the offset that named this body.
    fn descend(
        &mut self,
        offset: usize,
        name: &'a str,
        path: &mut Vec<&'a str>,
        depth: usize,
    ) -> Option<()> {
        if depth >= RAP_MAX_DEPTH || offset >= self.blob.len() || self.open.contains(&offset) {
            return Some(());
        }
        self.open.push(offset);
        path.push(name);
        let blob = self.blob;
        let result = self.walk(RapReader::new(&blob[offset..]), path, depth + 1);
        path.pop();
        self.open.pop();
        result
    }

    /// Read an array body. String elements are returned; numeric ones are
    /// skipped by their fixed width.
    fn array(&mut self, reader: &mut RapReader<'a>, depth: usize) -> Option<Vec<String>> {
        let count = reader.cint()?;
        if count > RAP_MAX_CHILDREN {
            return None;
        }
        let mut strings = Vec::new();
        for _ in 0..count {
            match reader.byte()? {
                0 => {
                    if let Some(text) = reader.cstring()? {
                        strings.push(text.to_string());
                    }
                }
                1 | 2 => reader.skip(4)?,
                3 if depth < RAP_MAX_DEPTH => {
                    self.array(reader, depth + 1)?;
                }
                _ => return None,
            }
        }
        Some(strings)
    }

    fn note_class(&mut self, path: &[&str], name: &str) {
        if path.len() != 1 || !path[0].eq_ignore_ascii_case("CfgPatches") {
            return;
        }
        if self.facts.cfgpatches_class.is_none() {
            self.facts.cfgpatches_class = Some(name.to_string());
        }
        if !self
            .facts
            .cfgpatches_classes
            .iter()
            .any(|seen| seen.eq_ignore_ascii_case(name))
        {
            self.facts.cfgpatches_classes.push(name.to_string());
        }
    }

    fn note_value(&mut self, path: &[&str], name: &str, value: Option<&str>) {
        let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
            return;
        };
        let key = name.strip_suffix("[]").unwrap_or(name);
        if in_section(path, "CfgPatches") {
            match key {
                "author" | "authorStr" => {
                    set_author(&mut self.facts.cfgpatches_author, Some(value.to_string()));
                }
                "url" if value.starts_with("http") => {
                    set_first(&mut self.facts.cfgpatches_url, Some(value.to_string()));
                }
                _ => {}
            }
        } else if in_section(path, "CfgMods") {
            match key {
                "name" => set_first(&mut self.facts.cfgmods_name, Some(value.to_string())),
                "author" | "authorStr" => {
                    set_author(&mut self.facts.cfgmods_author, Some(value.to_string()));
                }
                "url" if value.starts_with("http") => {
                    set_first(&mut self.facts.cfgmods_url, Some(value.to_string()));
                }
                _ => {}
            }
        }
    }

    fn note_array(&mut self, path: &[&str], name: &str, values: &[String]) {
        let key = name.strip_suffix("[]").unwrap_or(name);
        if !key.eq_ignore_ascii_case("requiredAddons") || !in_section(path, "CfgPatches") {
            return;
        }
        for value in values {
            if !value.is_empty() && !self.facts.required_addons.contains(value) {
                self.facts.required_addons.push(value.clone());
            }
        }
    }
}

/// Is the walk inside the named root class?
fn in_section(path: &[&str], section: &str) -> bool {
    path.first()
        .map(|root| root.eq_ignore_ascii_case(section))
        .unwrap_or(false)
}

/// Facts from the first `\0raP` blob in `data` that parses. The magic can
/// sit anywhere in the window, so the scan does not assume offset zero.
fn parse_rap_config(data: &[u8]) -> Option<RapFacts> {
    let mut from = 0usize;
    for _ in 0..RAP_MAX_BLOBS {
        let index = data
            .get(from..)?
            .windows(RAP_MAGIC.len())
            .position(|window| window == RAP_MAGIC)?;
        let start = from + index;
        from = start + RAP_MAGIC.len();
        if let Some(facts) = rap_facts_at(&data[start..]) {
            if !facts.is_empty() {
                return Some(facts);
            }
        }
    }
    None
}

/// Parse one blob that opens with the magic. The body follows the fixed
/// header, and every class offset is relative to the blob start.
fn rap_facts_at(blob: &[u8]) -> Option<RapFacts> {
    let mut reader = RapReader::new(blob);
    reader.skip(RAP_MAGIC.len())?;
    if reader.u32()? != 0 || reader.u32()? != 8 {
        return None;
    }
    reader.u32()?; // enum block offset
    let body = blob.get(RAP_HEADER_LEN..)?;
    let mut parser = RapParser {
        blob,
        facts: RapFacts::default(),
        classes: 0,
        open: Vec::new(),
    };
    parser.walk(RapReader::new(body), &mut Vec::new(), 0)?;
    Some(parser.facts)
}

/// Overwrite string-scan facts with the ones a real config carries.
fn merge_rap_facts(facts: &mut PboConfigFacts, rap: RapFacts) {
    if let Some(class) = rap.cfgpatches_class {
        facts.cfgpatches_class = Some(class);
    }
    if !rap.cfgpatches_classes.is_empty() {
        facts.cfgpatches_classes = rap.cfgpatches_classes;
    }
    set_author(&mut facts.cfgpatches_author, rap.cfgpatches_author);
    if let Some(url) = rap.cfgpatches_url.or(rap.cfgmods_url) {
        facts.cfgpatches_url = Some(url);
    }
    if !rap.required_addons.is_empty() {
        let mut addons = rap.required_addons;
        for addon in std::mem::take(&mut facts.required_addons) {
            if !addons.contains(&addon) {
                addons.push(addon);
            }
        }
        facts.required_addons = addons;
    }
    if let Some(name) = rap.cfgmods_name {
        facts.cfgmods_name = Some(name);
    }
    set_author(&mut facts.cfgmods_author, rap.cfgmods_author);
}

/// Parse the stored `config.bin` candidates: root first, then nested, up
/// to [`PBO_MAX_CONFIG_ENTRIES`]. `read` yields the entry body, `Ok(None)`
/// skips an unreadable entry and `Err` stops the scan.
fn rap_from_config_entries_with(
    entries: &[PboEntry],
    mut read: impl FnMut(&PboEntry) -> Result<Option<Vec<u8>>, ()>,
) -> Option<RapFacts> {
    let mut candidates: Vec<&PboEntry> = entries
        .iter()
        .filter(|entry| {
            entry.mime == 0
                && u64::from(entry.data_size) <= PBO_CONFIG_ENTRY_LIMIT
                && is_binary_config_entry(&entry.name)
        })
        .collect();
    candidates.sort_by_key(|entry| entry.name.contains(['\\', '/']) as u8);

    for entry in candidates.into_iter().take(PBO_MAX_CONFIG_ENTRIES) {
        match read(entry) {
            Err(()) => return None,
            Ok(Some(data)) => {
                if let Some(facts) = parse_rap_config(&data) {
                    return Some(facts);
                }
            }
            Ok(None) => {}
        }
    }
    None
}

/// A stored `config.bin` at any depth in the PBO.
fn is_binary_config_entry(name: &str) -> bool {
    name.rsplit(['\\', '/'])
        .next()
        .map(|stem| stem.eq_ignore_ascii_case("config.bin"))
        .unwrap_or(false)
}

/// An unpacked `config.cpp`, which the text scan reads as printable runs.
fn is_text_config_entry(name: &str) -> bool {
    name.rsplit(['\\', '/'])
        .next()
        .map(|stem| stem.eq_ignore_ascii_case("config.cpp"))
        .unwrap_or(false)
}

/// Parse the stored config entries of a PBO. A large pack puts its config
/// beyond the scan window, so the entry itself is read. The pack's own
/// config sits at the root; a nested one configures a single object.
fn rap_from_config_entries(path: &Path) -> Option<RapFacts> {
    use std::io::{Read, Seek, SeekFrom};

    let entries = pbo_entries(path);
    let mut file: Option<fs::File> = None;
    rap_from_config_entries_with(&entries, |entry| {
        if file.is_none() {
            file = Some(fs::File::open(path).ok().ok_or(())?);
        }
        let file = file.as_mut().ok_or(())?;
        let mut data = Vec::new();
        file.seek(SeekFrom::Start(entry.offset)).map_err(|_| ())?;
        match file
            .by_ref()
            .take(u64::from(entry.data_size))
            .read_to_end(&mut data)
        {
            Ok(_) => Ok(Some(data)),
            Err(_) => Ok(None),
        }
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn key_of_returns_trimmed_left_side() {
        assert_eq!(key_of("name = \"x\""), Some("name"));
        assert_eq!(key_of("  author=\"y\""), Some("author"));
        assert_eq!(key_of("no assignment"), None);
    }

    #[test]
    fn parse_mod_cpp_matches_exact_keys_only() {
        let dir = std::env::temp_dir().join("uksfta-pbo-modcpp");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mod.cpp");
        fs::write(
            &path,
            "namespace = \"wrong\"\nname = \"Right\"\nauthorName = \"wrong\"\nauthor = \"Auth\"\n",
        )
        .unwrap();
        let meta = parse_mod_cpp(&path);
        assert_eq!(meta.name, "Right");
        assert_eq!(meta.author, "Auth");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parse_meta_cpp_matches_exact_keys_only() {
        let dir = std::env::temp_dir().join("uksfta-pbo-metacpp");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("meta.cpp");
        fs::write(
            &path,
            "publishedid = \"1234567890\"\nname = \"Right\"\nnamespace = \"wrong\"\n",
        )
        .unwrap();
        let meta = parse_meta_cpp(&path);
        assert_eq!(meta.publishedid.as_deref(), Some("1234567890"));
        assert_eq!(meta.name, "Right");
        fs::remove_dir_all(&dir).unwrap();
    }

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
    fn pbo_prefix_extracts_canonical_path() {
        let dir = std::env::temp_dir().join("uksfta-prefix-test");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo(&f, "z\\ace\\addons\\grenades", b"data");
        assert_eq!(
            pbo_prefix(&f).unwrap(),
            "z\\ace\\addons\\grenades".to_string()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_prefix_no_prefix_returns_none() {
        let dir = std::env::temp_dir().join("uksfta-prefix-none");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        fs::write(&f, b"\x00sreVno prefix here").unwrap();
        assert_eq!(pbo_prefix(&f), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Write a fake PBO whose packed config contains the given text.
    fn write_pbo_with_config(path: &Path, config: &str) {
        let mut data = b"\x00sreV\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        data.extend_from_slice(b"prefix\x00test\x00");
        data.extend_from_slice(config.as_bytes());
        fs::write(path, data).unwrap();
    }

    #[test]
    fn pbo_config_facts_reads_cfgpatches_block() {
        let dir = std::env::temp_dir().join("uksfta-pbo-facts-patches");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("ade_core.pbo");
        write_pbo_with_config(
            &f,
            "class CfgPatches\n{\n    class ade_core\n    {\n        units[] = {};\n        requiredAddons[] = {\"Extended_EventHandlers\",\"ace_common\"};\n        author = \"[ADE]\";\n        displayName = \"Diving Mask (ADE)\";\n    };\n};\n",
        );

        let facts = pbo_config_facts(&f);

        assert_eq!(facts.cfgpatches_class.as_deref(), Some("ade_core"));
        assert_eq!(facts.cfgpatches_author.as_deref(), Some("[ADE]"));
        assert_eq!(facts.display_names, ["Diving Mask (ADE)"]);
        assert_eq!(
            facts.required_addons,
            ["Extended_EventHandlers", "ace_common"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_config_facts_collects_every_display_name() {
        let dir = std::env::temp_dir().join("uksfta-pbo-facts-displaynames");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo_with_config(
            &f,
            r#"class CfgPatches { class one { displayName = "Alpha"; displayName = "Bravo"; }; };"#,
        );

        assert_eq!(pbo_config_facts(&f).display_names, ["Alpha", "Bravo"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_config_facts_reads_cfgmods_name_and_author() {
        let dir = std::env::temp_dir().join("uksfta-pbo-facts-cfgmods");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo_with_config(
            &f,
            r#"class CfgMods { class BettIR { name = "BettIR Core"; author = "BettIR"; }; };"#,
        );

        let facts = pbo_config_facts(&f);

        assert_eq!(facts.cfgmods_name.as_deref(), Some("BettIR Core"));
        assert_eq!(facts.cfgmods_author.as_deref(), Some("BettIR"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_config_facts_extracts_str_tokens() {
        let dir = std::env::temp_dir().join("uksfta-pbo-facts-strtokens");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo_with_config(&f, r#"author = "$STR_RHSUSF_AUTHOR_FULL";"#);

        assert_eq!(pbo_config_facts(&f).str_tokens, ["$STR_RHSUSF_AUTHOR_FULL"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_config_facts_empty_for_compressed_body() {
        let dir = std::env::temp_dir().join("uksfta-pbo-facts-compressed");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        let mut payload = b"Cprs".to_vec();
        payload.extend((0u8..=255).filter(|byte| !(0x20..=0x7e).contains(byte)));
        let mut data = b"\x00sreV\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec();
        data.extend_from_slice(b"prefix\x00test\x00");
        data.extend_from_slice(&payload);
        fs::write(&f, data).unwrap();

        assert_eq!(pbo_config_facts(&f), PboConfigFacts::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_fingerprint_is_hex_and_stable() {
        let dir = std::env::temp_dir().join("uksfta-pbo-fingerprint-stable");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo_with_config(&f, "class CfgPatches { class one {}; };");

        let first = pbo_fingerprint(&f).unwrap();
        let second = pbo_fingerprint(&f).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_fingerprint_ignores_the_middle_of_a_large_file() {
        let dir = std::env::temp_dir().join("uksfta-pbo-fingerprint-window");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        let mut data = vec![b'a'; 200 * 1024];
        fs::write(&f, &data).unwrap();
        let baseline = pbo_fingerprint(&f).unwrap();

        data[100 * 1024] = b'b';
        fs::write(&f, &data).unwrap();
        assert_eq!(pbo_fingerprint(&f).unwrap(), baseline);

        data[10] = b'b';
        fs::write(&f, &data).unwrap();
        assert_ne!(pbo_fingerprint(&f).unwrap(), baseline);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_fingerprint_changes_with_length() {
        let dir = std::env::temp_dir().join("uksfta-pbo-fingerprint-length");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        fs::write(&f, b"short").unwrap();
        let short = pbo_fingerprint(&f).unwrap();

        fs::write(&f, b"short ").unwrap();

        assert_ne!(pbo_fingerprint(&f).unwrap(), short);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_fingerprint_missing_file_returns_none() {
        let missing = std::env::temp_dir().join("uksfta-pbo-fingerprint-missing.pbo");
        assert_eq!(pbo_fingerprint(&missing), None);
    }

    #[test]
    fn pbo_config_facts_prefers_bracketed_author() {
        let dir = std::env::temp_dir().join("uksfta-pbo-facts-bracket-author");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        write_pbo_with_config(
            &f,
            r#"class CfgPatches { class one { author = "SovjetPwr"; }; class two { author = "[ADE]"; }; };"#,
        );

        assert_eq!(
            pbo_config_facts(&f).cfgpatches_author.as_deref(),
            Some("[ADE]")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Build a PBO header with a properties entry, the given named
    /// entries, a terminator and the data block in header order.
    pub(crate) fn write_pbo_entries(path: &Path, entries: &[(&str, u32, &[u8])]) {
        let properties = b"prefix\0test_prefix\0\0";
        let mut bytes = Vec::new();
        bytes.push(0); // empty properties name
        bytes.extend(PBO_PROPERTIES_MIME.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend((properties.len() as u32).to_le_bytes());
        bytes.extend(properties);
        for (name, mime, data) in entries {
            bytes.extend(name.as_bytes());
            bytes.push(0);
            bytes.extend(mime.to_le_bytes());
            bytes.extend((data.len() as u32).to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            bytes.extend((data.len() as u32).to_le_bytes());
        }
        bytes.extend([0u8; 21]); // empty name plus five zero fields
        for (_, _, data) in entries {
            bytes.extend(*data);
        }
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn pbo_entries_reads_stored_and_compressed() {
        let dir = std::env::temp_dir().join("uksfta-pbo-entries");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        let config: &[u8] = b"class CfgPatches {};";
        write_pbo_entries(
            &f,
            &[
                ("config.cpp", 0, config),
                ("config.bin", PBO_CPRS_MIME, b"\x00compressed"),
                ("secret.bin", PBO_ENCR_MIME, b"encrypted"),
            ],
        );

        let entries = pbo_entries(&f);

        assert_eq!(
            entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
            ["config.cpp", "config.bin", "secret.bin"]
        );
        assert_eq!(entries[0].mime, 0);
        assert_eq!(entries[0].data_size, config.len() as u32);
        assert_eq!(entries[1].mime, PBO_CPRS_MIME);
        assert_eq!(entries[2].mime, PBO_ENCR_MIME);
        assert!(entries
            .windows(2)
            .all(|pair| pair[0].offset < pair[1].offset));

        let sources = pbo_text_sources(&f);
        assert_eq!(
            sources
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>(),
            ["config.cpp", "config.bin"]
        );
        assert_eq!(sources[0].1, b"class CfgPatches {};");
        assert_eq!(sources[1].1, b"\x00compressed");

        let stored = pbo_stored_sources_capped(&f).0;
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].0, "config.cpp");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_entries_malformed_header_is_empty() {
        let dir = std::env::temp_dir().join("uksfta-pbo-entries-malformed");
        fs::create_dir_all(&dir).unwrap();

        let huge = dir.join("huge.pbo");
        let mut bytes = Vec::new();
        bytes.extend(b"config.cpp");
        bytes.push(0);
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(u32::MAX.to_le_bytes()); // data size beyond the file
        bytes.extend([0u8; 21]);
        fs::write(&huge, bytes).unwrap();
        assert!(pbo_entries(&huge).is_empty());

        let truncated = dir.join("truncated.pbo");
        fs::write(&truncated, b"short").unwrap();
        assert!(pbo_entries(&truncated).is_empty());
        assert!(pbo_text_sources(&truncated).is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }

    /// One entry in a synthetic binarised config body.
    enum RapEntry {
        /// Class whose body follows the entry list.
        Class(String),
        /// Class with an offset written by hand, so a body can point at
        /// itself.
        ClassAt(String, u32),
        /// Value entry, subtype 0, carrying text.
        Text(&'static str, &'static str),
        /// Value entry with a subtype the harness does not know, no payload.
        Unknown(u8, &'static str),
        /// Array entry of string elements.
        Strings(&'static str, Vec<&'static str>),
    }

    /// Build a binarised config tree. Class offsets are absolute in the
    /// finished body, so encoding walks the tree from its base offset.
    #[derive(Default)]
    struct RapBody {
        parent: String,
        entries: Vec<RapEntry>,
        bodies: Vec<RapBody>,
    }

    impl RapBody {
        fn new(parent: &str) -> Self {
            Self {
                parent: parent.to_string(),
                entries: Vec::new(),
                bodies: Vec::new(),
            }
        }

        fn class(&mut self, name: &str, body: RapBody) -> &mut Self {
            self.bodies.push(body);
            self.entries.push(RapEntry::Class(name.to_string()));
            self
        }

        fn class_at(&mut self, name: &str, offset: u32) -> &mut Self {
            self.entries
                .push(RapEntry::ClassAt(name.to_string(), offset));
            self
        }

        fn text(&mut self, name: &'static str, value: &'static str) -> &mut Self {
            self.entries.push(RapEntry::Text(name, value));
            self
        }

        fn unknown(&mut self, subtype: u8, name: &'static str) -> &mut Self {
            self.entries.push(RapEntry::Unknown(subtype, name));
            self
        }

        fn strings(&mut self, name: &'static str, values: &[&'static str]) -> &mut Self {
            self.entries.push(RapEntry::Strings(name, values.to_vec()));
            self
        }

        /// Encode this body as it sits at `base` in the whole config body.
        fn encode_at(&self, base: usize) -> Vec<u8> {
            let head = self.parent.len() + 1 + cint_len(self.entries.len());
            let entries_len: usize = self.entries.iter().map(encoded_len).sum();
            let mut body = Vec::new();
            body.extend_from_slice(self.parent.as_bytes());
            body.push(0);
            push_cint(&mut body, self.entries.len());
            let mut next = base + head + entries_len;
            for entry in &self.entries {
                match entry {
                    RapEntry::Class(name) => {
                        body.push(0);
                        body.extend_from_slice(name.as_bytes());
                        body.push(0);
                        body.extend_from_slice(&(next as u32).to_le_bytes());
                    }
                    RapEntry::ClassAt(name, offset) => {
                        body.push(0);
                        body.extend_from_slice(name.as_bytes());
                        body.push(0);
                        body.extend_from_slice(&offset.to_le_bytes());
                    }
                    RapEntry::Text(name, value) => {
                        body.push(1);
                        body.push(0);
                        body.extend_from_slice(name.as_bytes());
                        body.push(0);
                        body.extend_from_slice(value.as_bytes());
                        body.push(0);
                    }
                    RapEntry::Unknown(subtype, name) => {
                        body.push(1);
                        body.push(*subtype);
                        body.extend_from_slice(name.as_bytes());
                        body.push(0);
                    }
                    RapEntry::Strings(name, values) => {
                        body.push(2);
                        body.extend_from_slice(name.as_bytes());
                        body.push(0);
                        push_cint(&mut body, values.len());
                        for value in values {
                            body.push(0);
                            body.extend_from_slice(value.as_bytes());
                            body.push(0);
                        }
                    }
                }
            }
            for child in &self.bodies {
                let bytes = child.encode_at(next);
                next += bytes.len();
                body.extend_from_slice(&bytes);
            }
            body
        }
    }

    fn encoded_len(entry: &RapEntry) -> usize {
        match entry {
            RapEntry::Class(name) | RapEntry::ClassAt(name, _) => 1 + name.len() + 1 + 4,
            RapEntry::Text(name, value) => 2 + name.len() + 1 + value.len() + 1,
            RapEntry::Unknown(_, name) => 2 + name.len() + 1,
            RapEntry::Strings(name, values) => {
                1 + name.len()
                    + 1
                    + cint_len(values.len())
                    + values
                        .iter()
                        .map(|value| 1 + value.len() + 1)
                        .sum::<usize>()
            }
        }
    }

    fn cint_len(value: usize) -> usize {
        let mut value = value;
        let mut len = 1;
        while value > 0x7f {
            value >>= 7;
            len += 1;
        }
        len
    }

    fn push_cint(out: &mut Vec<u8>, value: usize) {
        let mut value = value;
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                return;
            }
        }
    }

    /// Wrap a root body in the `\0raP` header. Class offsets are relative
    /// to the blob, so the body is encoded at the header length.
    fn rap_blob(root: &RapBody) -> Vec<u8> {
        let mut blob = Vec::new();
        blob.extend_from_slice(b"\0raP");
        blob.extend_from_slice(&0u32.to_le_bytes());
        blob.extend_from_slice(&8u32.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes()); // enum offset
        blob.extend_from_slice(&root.encode_at(RAP_HEADER_LEN));
        blob
    }

    /// Root body holding `CfgPatches` with one patch class.
    fn rap_root(patch: RapBody) -> Vec<u8> {
        let mut patches = RapBody::new("CfgPatches");
        patches.class("ade_core", patch);
        let mut root = RapBody::new("");
        root.class("CfgPatches", patches);
        rap_blob(&root)
    }

    #[test]
    fn rap_parses_cfgpatches_and_required_addons() {
        let mut patch = RapBody::new("ade_core");
        patch.text("author", "[ADE]");
        patch.strings(
            "requiredAddons[]",
            &["Extended_EventHandlers", "ace_common"],
        );

        let facts = parse_rap_config(&rap_root(patch)).expect("facts");

        assert_eq!(facts.cfgpatches_class.as_deref(), Some("ade_core"));
        assert_eq!(facts.cfgpatches_author.as_deref(), Some("[ADE]"));
        assert_eq!(
            facts.required_addons,
            ["Extended_EventHandlers", "ace_common"]
        );
    }

    #[test]
    fn rap_parses_cfgmods_name() {
        let mut entry = RapBody::new("BettIR");
        entry.text("name", "BettIR Core");
        entry.text("author", "BettIR");
        let mut mods = RapBody::new("CfgMods");
        mods.class("BettIR", entry);
        let mut root = RapBody::new("");
        root.class("CfgMods", mods);

        let facts = parse_rap_config(&rap_blob(&root)).expect("facts");

        assert_eq!(facts.cfgmods_name.as_deref(), Some("BettIR Core"));
        assert_eq!(facts.cfgmods_author.as_deref(), Some("BettIR"));
    }

    #[test]
    fn rap_rejects_truncated_input_without_panic() {
        let mut patch = RapBody::new("ade_core");
        patch.text("author", "[ADE]");
        patch.strings("requiredAddons[]", &["ace_common"]);
        let blob = rap_root(patch);

        for length in 0..blob.len() {
            let _ = parse_rap_config(&blob[..length]);
        }
        assert!(parse_rap_config(&blob[..blob.len() - 3]).is_none());
        assert!(parse_rap_config(&blob).is_some());
    }

    #[test]
    fn rap_guards_class_cycles() {
        let mut patch = RapBody::new("ade_core");
        patch.text("author", "[ADE]");
        let mut patches = RapBody::new("CfgPatches");
        patches.class("ade_core", patch);
        let mut root = RapBody::new("");
        root.class("CfgPatches", patches);
        root.class_at("Self", RAP_HEADER_LEN as u32); // the root body points at itself

        let facts = parse_rap_config(&rap_blob(&root)).expect("facts under a cycle");

        assert_eq!(facts.cfgpatches_class.as_deref(), Some("ade_core"));
        assert_eq!(facts.cfgpatches_author.as_deref(), Some("[ADE]"));
    }

    #[test]
    fn rap_skips_unknown_subtype() {
        let mut patch = RapBody::new("ade_core");
        patch.unknown(6, "futureValue");
        patch.text("author", "[ADE]");

        let facts = parse_rap_config(&rap_root(patch)).expect("facts");

        assert_eq!(facts.cfgpatches_author.as_deref(), Some("[ADE]"));
    }

    #[test]
    fn pbo_config_facts_prefers_binarised_config() {
        let dir = std::env::temp_dir().join("uksfta-pbo-facts-rap");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("ade_core.pbo");

        let mut patch = RapBody::new("ade_core");
        patch.text("author", "[ADE]");
        let config = rap_root(patch);
        write_pbo_entries(
            &f,
            &[
                (
                    "config.cpp",
                    0,
                    b"class CfgPatches { class ade_stale { author = \"Stale\"; }; };",
                ),
                ("config.bin", 0, &config),
            ],
        );

        let facts = pbo_config_facts(&f);

        assert_eq!(facts.cfgpatches_class.as_deref(), Some("ade_core"));
        assert_eq!(facts.cfgpatches_author.as_deref(), Some("[ADE]"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_provided_classes_reads_every_text_class() {
        let dir = std::env::temp_dir().join("uksfta-pbo-provided-text");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        let config: &[u8] = br#"class CfgPatches { class alpha_main {}; class beta_extra {}; };"#;
        write_pbo_entries(&f, &[("config.cpp", 0, config)]);

        assert_eq!(pbo_provided_classes(&f), ["alpha_main", "beta_extra"]);

        let facts = pbo_config_facts(&f);
        assert_eq!(facts.cfgpatches_class.as_deref(), Some("alpha_main"));
        assert_eq!(facts.cfgpatches_classes, ["alpha_main", "beta_extra"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_provided_classes_prefers_binarised_config() {
        let dir = std::env::temp_dir().join("uksfta-pbo-provided-rap");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");

        let mut patches = RapBody::new("CfgPatches");
        patches.class("alpha_main", RapBody::new("alpha_main"));
        patches.class("beta_extra", RapBody::new("beta_extra"));
        let mut root = RapBody::new("");
        root.class("CfgPatches", patches);
        let config = rap_blob(&root);
        let stale: &[u8] = br#"class CfgPatches { class stale_name {}; };"#;
        write_pbo_entries(&f, &[("config.cpp", 0, stale), ("config.bin", 0, &config)]);

        assert_eq!(pbo_provided_classes(&f), ["alpha_main", "beta_extra"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pbo_provided_classes_is_empty_without_a_config() {
        let dir = std::env::temp_dir().join("uksfta-pbo-provided-empty");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("mod.pbo");
        let config: &[u8] = b"class CfgPatches {};";
        write_pbo_entries(&f, &[("config.cpp", 0, config)]);
        assert!(pbo_provided_classes(&f).is_empty());

        let truncated = dir.join("truncated.pbo");
        fs::write(&truncated, b"short").unwrap();
        assert!(pbo_provided_classes(&truncated).is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn closure_facts_match_config_facts_for_the_two_fields() {
        let dir = std::env::temp_dir().join("uksfta-pbo-closure-facts");
        fs::create_dir_all(&dir).unwrap();

        let text = dir.join("text.pbo");
        let config: &[u8] = br#"class CfgPatches { class alpha_main { requiredAddons[] = {"A3_Data_F", "ace_common"}; }; class beta_extra {}; };"#;
        write_pbo_entries(&text, &[("config.cpp", 0, config)]);

        let mut alpha = RapBody::new("alpha_main");
        alpha.strings("requiredAddons[]", &["A3_Data_F", "cba_main"]);
        let mut patches = RapBody::new("CfgPatches");
        patches.class("alpha_main", alpha);
        let mut root = RapBody::new("");
        root.class("CfgPatches", patches);
        let packed = rap_blob(&root);
        let stale: &[u8] =
            br#"class CfgPatches { class stale_name { requiredAddons[] = {"stale_dep"}; }; };"#;
        let binary = dir.join("binary.pbo");
        write_pbo_entries(
            &binary,
            &[("config.cpp", 0, stale), ("config.bin", 0, &packed)],
        );

        for path in [&text, &binary] {
            let facts = pbo_config_facts(path);
            let closure = pbo_closure_facts(path);
            assert_eq!(closure.cfgpatches_classes, facts.cfgpatches_classes);
            assert_eq!(closure.required_addons, facts.required_addons);
        }
        assert_eq!(
            pbo_closure_facts(&text).required_addons,
            ["A3_Data_F", "ace_common"]
        );
        assert_eq!(
            pbo_closure_facts(&binary).cfgpatches_classes,
            ["alpha_main"]
        );
        assert_eq!(
            pbo_closure_facts(&binary).required_addons,
            ["A3_Data_F", "cba_main", "stale_dep"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The marker scan against the printable-run scan it replaced.
    mod scan_tests {
        use super::super::*;
        use super::{rap_blob, write_pbo, RapBody};

        /// The per-byte printable-run scan this change replaced, kept
        /// verbatim so the two can be compared byte for byte.
        fn readable_strings_per_byte(data: &[u8]) -> String {
            let mut text = String::new();
            let mut run: Vec<u8> = Vec::new();
            for &byte in data {
                match byte {
                    b'{' | b'}' => {
                        push_run(&mut run, &mut text);
                        text.push(byte as char);
                        text.push('\n');
                    }
                    0x20..=0x7e => run.push(byte),
                    _ => push_run(&mut run, &mut text),
                }
            }
            push_run(&mut run, &mut text);
            text
        }

        fn push_run(run: &mut Vec<u8>, text: &mut String) {
            if run.len() >= RUN_MIN {
                text.push_str(&String::from_utf8_lossy(run));
                text.push('\n');
            }
            run.clear();
        }

        /// The closure fields the text scan produced.
        fn closure_text_facts(text: &str) -> PboConfigFacts {
            let mut facts = PboConfigFacts {
                required_addons: required_addons(text),
                ..PboConfigFacts::default()
            };
            scan_sections(text, &mut facts);
            facts
        }

        /// The whole old closure path for one window and PBO.
        fn closure_facts_per_byte(head: &[u8], path: Option<&Path>) -> PboClosureFacts {
            let mut facts = closure_text_facts(&readable_strings_per_byte(head));
            if let Some(rap) = path
                .and_then(rap_from_config_entries)
                .or_else(|| parse_rap_config(head))
            {
                merge_rap_facts(&mut facts, rap);
            }
            PboClosureFacts {
                cfgpatches_classes: facts.cfgpatches_classes,
                required_addons: facts.required_addons,
            }
        }

        /// Compare both scans over one window, without the stored
        /// `config.bin` path.
        fn assert_same_window(head: &[u8]) {
            let per_byte = readable_strings_per_byte(head);
            assert_eq!(readable_strings(head), per_byte, "text differs");
            let old = closure_text_facts(&per_byte);
            let new = closure_scan(head);
            assert_eq!(
                new.cfgpatches_classes,
                old.cfgpatches_classes,
                "classes differ for {:?}",
                String::from_utf8_lossy(head)
            );
            assert_eq!(
                new.required_addons,
                old.required_addons,
                "addons differ for {:?}",
                String::from_utf8_lossy(head)
            );
        }

        /// Compare the whole `pbo_closure_facts` path over one PBO file.
        fn assert_same_pbo(path: &Path) -> PboClosureFacts {
            let head = read_capped(path, PBO_SCAN_LIMIT).unwrap_or_default();
            let old = closure_facts_per_byte(&head, Some(path));
            let new = pbo_closure_facts(path);
            assert_eq!(new, old, "closure differs for {}", path.display());
            new
        }

        fn scratch(name: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(name);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn text_config_matches_the_per_byte_scan() {
            let dir = scratch("uksfta-scan-text");
            let f = dir.join("ade_core.pbo");
            let config: &[u8] = b"class CfgMods { name = \"ADE Core\"; author = \"ADE\"; };\n\
                class CfgPatches {\n    class ade_core {\n        units[] = {};\n\
                requiredAddons[] = {\n            \"A3_Data_F\",\n            \"ace_common\"\n        };\n\
                author = \"[ADE]\";\n    };\n    class ade_gear {\n        requiredAddons[] = {\"ade_core\"};\n    };\n};\n";
            write_pbo(&f, "z\\ade\\addons\\ade_core", config);

            let facts = assert_same_pbo(&f);
            assert_eq!(facts.cfgpatches_classes, ["ade_core", "ade_gear"]);
            assert_eq!(
                facts.required_addons,
                ["A3_Data_F", "ace_common", "ade_core"]
            );
            fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        fn binary_config_matches_the_per_byte_scan() {
            let dir = scratch("uksfta-scan-binary");
            let f = dir.join("alpha.pbo");
            let mut alpha = RapBody::new("alpha_main");
            alpha.strings("requiredAddons[]", &["A3_Data_F", "cba_main"]);
            let mut patches = RapBody::new("CfgPatches");
            patches.class("alpha_main", alpha);
            let mut root = RapBody::new("");
            root.class("CfgPatches", patches);
            let packed = rap_blob(&root);
            // Printable noise before the blob: it must not add a class.
            let mut payload = b"class CfgPatches { class noise { }; };".to_vec();
            payload.push(0);
            payload.extend_from_slice(&packed);
            write_pbo(&f, "z\\alpha\\addons\\alpha_main", &payload);

            let facts = assert_same_pbo(&f);
            assert_eq!(facts.cfgpatches_classes, ["alpha_main"]);
            assert_eq!(facts.required_addons, ["A3_Data_F", "cba_main"]);
            fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        fn config_past_the_window_matches_the_per_byte_scan() {
            let dir = scratch("uksfta-scan-past-window");
            let f = dir.join("late.pbo");
            let mut payload = vec![0xaa_u8; PBO_SCAN_LIMIT as usize + 1024];
            payload.extend_from_slice(
                b"class CfgPatches { class late_main { requiredAddons[] = {\"late_dep\"}; }; };",
            );
            write_pbo(&f, "z\\late\\addons\\late_main", &payload);

            let facts = assert_same_pbo(&f);
            assert!(facts.cfgpatches_classes.is_empty());
            assert!(facts.required_addons.is_empty());
            fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        fn mixed_binary_noise_matches_the_per_byte_scan() {
            let mut noise = Vec::new();
            for round in 0..8u8 {
                noise.extend((0u8..=255).filter(|byte| !(0x20..=0x7e).contains(byte)));
                noise.extend_from_slice(b"\x00\x01 class CfgPatches { class ");
                noise.extend_from_slice(&[0xff, 0xfe, 0x00, round]);
                noise.extend_from_slice(
                    b"noise_cfg { requiredAddons[] = {\"A3_Data_F\",\r\n\"cba_main\"}; \
                      units[] = {class Decoy}; }; };",
                );
                noise.extend_from_slice(&[0x7f, 0x80, 0x00]);
            }
            write_scratch_and_compare(&noise);
        }

        /// Write the window as the body of a PBO and compare the scans.
        fn write_scratch_and_compare(head: &[u8]) {
            let dir = scratch("uksfta-scan-scratch");
            let f = dir.join("scratch.pbo");
            write_pbo(&f, "z\\scratch\\addons\\scratch", head);
            assert_same_window(head);
            assert_same_pbo(&f);
            fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        fn empty_window_matches_the_per_byte_scan() {
            assert_same_window(&[]);
            assert_same_window(b"\0\0\0\0");
            assert!(closure_scan(&[]).cfgpatches_classes.is_empty());
            assert!(closure_scan(&[]).required_addons.is_empty());
        }

        #[test]
        fn edge_markers_match_the_per_byte_scan() {
            let cases: &[&[u8]] = &[
                b"class ",
                b"class CfgPatches",
                b"class CfgPatches { class a {}; class b {}; };",
                b"units[] = {class Decoy}; class CfgPatches { class Real {}; };",
                b"class CfgPatches { class a { class Nested {}; }; };",
                b"class CfgMods { name = \"x\"; }; class CfgPatches { class p {}; };",
                b"requiredAddons[] = {",
                b"requiredAddons[]={\"A\",\"B\"}; requiredAddons[] = {\"B\",\"C\"};",
                b"text = \"requiredAddons[] = {inside}\";",
                b"requiredAddons[] = {};",
                b"requiredAddons[] = { \"A\",\0\0\"B\" };",
                b"subclass CfgPatches { class a {}; };",
                b"x class CfgPatches { class a {}; };",
                b"class CfgPatches{class a{}};",
                b"class \"CfgPatches\" { class a {}; };",
                b"class CfgPatches class Other { class a {}; };",
                b"class CfgPatches { ; class a {};; };",
                b"class CfgPatches {\r\n    class a {\r\n        requiredAddons[] = {\"A\"};\r\n    };\r\n};",
                b"}}class CfgPatches{class a{};}",
                b"class CfgPatches { class a { requiredAddons[] = {\"x\"} }; };",
                b"ssssclass CfgPatches { class a {}; };",
            ];
            for case in cases {
                assert_same_window(case);
            }
        }

        #[test]
        fn every_byte_value_matches_the_per_byte_scan() {
            let mut data: Vec<u8> = (0u8..=255).collect();
            data.extend((0u8..=255).rev());
            assert_same_window(&data);

            // Run boundaries at every offset around the eight-byte test.
            for split in 0..16usize {
                let mut data = vec![b'a'; split];
                data.extend_from_slice(&[0x00, b'x']);
                data.extend_from_slice(b"class CfgPatches { class a {}; };");
                assert_same_window(&data);
            }
        }

        #[test]
        fn printable_word_test_is_exact() {
            for byte in 0u8..=255 {
                let word = [byte; 8];
                assert_eq!(
                    all_run_bytes(&word),
                    (0x20..=0x7e).contains(&byte),
                    "byte {byte:#04x}"
                );
            }
            // One non-printable byte in each lane must be found.
            for lane in 0..8usize {
                let mut word = [b'z'; 8];
                word[lane] = 0x1f;
                assert!(!all_run_bytes(&word), "lane {lane}");
                word[lane] = 0x7f;
                assert!(!all_run_bytes(&word), "lane {lane}");
            }
        }
    }

    /// The closure pipeline from before the shared front read: a fixed
    /// 4 MB head, then a fresh open for the stored config. Every layout
    /// must still produce these facts.
    fn reference_closure_facts(path: &Path) -> PboClosureFacts {
        let Some(head) = read_capped(path, PBO_SCAN_LIMIT) else {
            return PboClosureFacts::default();
        };
        let text = readable_strings(&head);
        let rap = rap_from_config_entries(path).or_else(|| parse_rap_config(&head));
        let facts = base_facts(&text, rap);
        PboClosureFacts {
            cfgpatches_classes: facts.cfgpatches_classes,
            required_addons: facts.required_addons,
        }
    }

    #[test]
    fn closure_facts_match_the_whole_head_reader_on_every_layout() {
        let dir = std::env::temp_dir().join("uksfta-pbo-closure-front");
        fs::create_dir_all(&dir).unwrap();

        let text: &[u8] = br#"class CfgPatches { class alpha_main { requiredAddons[] = {"A3_Data_F", "ace_common"}; }; };"#;
        let mut patch = RapBody::new("alpha_main");
        patch.strings("requiredAddons[]", &["A3_Data_F", "cba_main"]);
        let mut patches = RapBody::new("CfgPatches");
        patches.class("alpha_main", patch);
        let mut root = RapBody::new("");
        root.class("CfgPatches", patches);
        let packed = rap_blob(&root);
        let payload = vec![0xa5u8; 200_000];

        let mut cases: Vec<PathBuf> = Vec::new();
        let mut write = |name: &str, entries: &[(&str, u32, &[u8])]| {
            let path = dir.join(name);
            write_pbo_entries(&path, entries);
            cases.push(path);
        };
        write("text.pbo", &[("config.cpp", 0, text)]);
        write("binary.pbo", &[("config.bin", 0, &packed)]);
        write(
            "both.pbo",
            &[("config.cpp", 0, text), ("config.bin", 0, &packed)],
        );
        write(
            "late_text.pbo",
            &[("models/big.p3d", 0, &payload), ("config.cpp", 0, text)],
        );
        write(
            "late_binary.pbo",
            &[("models/big.p3d", 0, &payload), ("config.bin", 0, &packed)],
        );
        write("nested.pbo", &[("z\\thing\\config.bin", 0, &packed)]);
        write("compressed.pbo", &[("config.bin", PBO_CPRS_MIME, &packed)]);

        // An entry table past the first read: the front has to grow.
        let names: Vec<String> = (0..4_000)
            .map(|index| format!("data/file{index}.txt"))
            .collect();
        let bodies: Vec<&[u8]> = (0..4_000).map(|_| b"x".as_slice()).collect();
        let wide: Vec<(&str, u32, &[u8])> = names
            .iter()
            .map(String::as_str)
            .zip(bodies)
            .map(|(name, body)| (name, 0, body))
            .collect();
        let path = dir.join("wide.pbo");
        write_pbo_entries(&path, &wide);

        for path in &cases {
            assert_eq!(
                pbo_closure_facts(path),
                reference_closure_facts(path),
                "{}",
                path.display()
            );
        }

        let wide = reference_closure_facts(&path);
        assert_eq!(pbo_closure_facts(&path), wide);
        assert!(pbo_entries(&path).len() == 4_000);

        // Unreadable and malformed files yield the same empty facts.
        let empty = dir.join("empty.pbo");
        fs::write(&empty, b"").unwrap();
        let short = dir.join("short.pbo");
        fs::write(&short, b"short").unwrap();
        let missing = dir.join("missing.pbo");
        for path in [&empty, &short, &missing] {
            assert_eq!(
                pbo_closure_facts(path),
                reference_closure_facts(path),
                "{}",
                path.display()
            );
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod cba_tests {
    use super::cba_keybind_names;

    #[test]
    fn cba_keybind_names_reads_category() {
        let text = "[\"Advanced Diving Environment\",\"adeVarDiveComputerVisible\", \"Toggle Dive Computer\", {}, {}, [0x3e, [true, false, false]]] call CBA_fnc_addKeybind;";

        assert_eq!(cba_keybind_names(text), ["Advanced Diving Environment"]);
    }

    #[test]
    fn cba_keybind_names_rejects_identifiers() {
        assert!(
            cba_keybind_names("[\"some_identifier\",\"x\"] call CBA_fnc_addKeybind;").is_empty()
        );
    }
}

/// The entry-scan and one-read paths, checked against the behaviour they
/// replaced.
#[cfg(test)]
mod read_tests {
    use super::tests::write_pbo_entries;
    use super::*;

    /// The scan this change replaced: every stored entry read in full,
    /// then filtered to authored text, with every name kept.
    fn keybind_names_by_full_scan(path: &Path, head: &str) -> Vec<String> {
        let mut names = cba_keybind_names(head);
        if names.is_empty() {
            let (sources, _) = pbo_stored_sources_capped(path);
            for (name, bytes) in sources {
                if !is_authored_text(&name) {
                    continue;
                }
                names.extend(cba_keybind_names(&readable_strings(&bytes)));
            }
        }
        names.sort();
        names.dedup();
        names
    }

    #[test]
    fn keybind_names_skip_binary_entries_and_match_the_full_scan() {
        let dir = std::env::temp_dir().join("uksfta-pbo-keybind-entries");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("ade.pbo");

        // Five MiB of model data puts the keybind code past the 4 MiB
        // head window, so only the entry scan can find it. A marker
        // inside the binary body must not become a category.
        let mut model = vec![0xaa_u8; 5 * 1024 * 1024];
        model.extend_from_slice(b"[\"Decoy Category\",\"x\"] call CBA_fnc_addKeybind;");
        let keybind: &[u8] =
            b"[\"Advanced Diving Environment\",\"adeVar\"] call CBA_fnc_addKeybind;";
        // A second file registers another category; both must be kept.
        let second: &[u8] = b"[\"ADE Diving Gear\",\"adeVar2\"] call CBA_fnc_addKeybind;";
        write_pbo_entries(
            &f,
            &[
                ("models/hull.pak", 0, &model),
                ("scripts/keys.sqf", 0, keybind),
                ("scripts/gear.sqf", 0, second),
            ],
        );

        let content = PboContent::read(&f);
        let head = readable_strings(&content.head);
        assert!(!head.contains("CBA_fnc_addKeybind"));

        let expected = keybind_names_by_full_scan(&f, &head);
        assert_eq!(expected, ["ADE Diving Gear", "Advanced Diving Environment"]);
        assert_eq!(content.config_facts().cba_keybind_names, expected);
        assert_eq!(pbo_config_facts(&f).cba_keybind_names, expected);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn content_config_facts_match_the_path_reader() {
        let dir = std::env::temp_dir().join("uksfta-pbo-content-facts");
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("ade_core.pbo");

        let config: &[u8] = br#"class CfgMods { name = "ADE Core"; author = "ADE"; };
            class CfgPatches { class ade_core { units[] = {};
            requiredAddons[] = {"ace_common"}; author = "[ADE]";
            url = "https://steamcommunity.com/sharedfiles/filedetails/?id=1234";
            displayName = "Diving Core"; }; };"#;
        write_pbo_entries(
            &f,
            &[
                ("config.cpp", 0, config),
                ("docs/readme.txt", 0, b"Advanced Diving Environment"),
                ("models/hull.pak", 0, &[0xaa; 64]),
            ],
        );

        let content = PboContent::read(&f);
        assert_eq!(content.config_facts(), pbo_config_facts(&f));
        fs::remove_dir_all(&dir).unwrap();
    }
}
