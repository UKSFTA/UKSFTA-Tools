use std::fs;
use std::path::Path;

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
fn key_of(line: &str) -> Option<&str> {
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
const PBO_CPRS_MIME: u32 = 0x4370_7273; // Cprs
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
    use std::io::{BufReader, Seek};

    // Names are read a byte at a time, so the file is buffered.
    let file = fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let mut file = BufReader::new(file);

    // Properties header: empty name, mime `Vers`, four sizes. Its data is
    // inline and has no stored length: `key\0value\0...\0`.
    let name = read_cstring(&mut file, PBO_MAX_NAME)?;
    let mime = read_le_u32(&mut file)?;
    if !name.is_empty() || mime != PBO_PROPERTIES_MIME {
        return None;
    }
    for _ in 0..4 {
        read_le_u32(&mut file)?;
    }
    loop {
        let key = read_cstring(&mut file, PBO_MAX_NAME)?;
        if key.is_empty() {
            break;
        }
        read_cstring(&mut file, PBO_MAX_NAME)?;
    }

    let mut raw: Vec<RawPboEntry> = Vec::new();
    loop {
        if raw.len() >= PBO_MAX_HEADER_ENTRIES {
            return None;
        }
        let name = read_cstring(&mut file, PBO_MAX_NAME)?;
        let mime = read_le_u32(&mut file)?;
        let original_size = read_le_u32(&mut file)?;
        let reserved = read_le_u32(&mut file)?;
        let timestamp = read_le_u32(&mut file)?;
        let data_size = read_le_u32(&mut file)?;

        // Terminator: empty name plus twenty zero bytes.
        if name.is_empty()
            && mime == 0
            && original_size == 0
            && reserved == 0
            && timestamp == 0
            && data_size == 0
        {
            let mut offset = file.stream_position().ok()?;
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
            return Some(entries);
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
    use std::io::{Read, Seek, SeekFrom};

    let entries = pbo_entries(path);
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
        sources.push((entry.name, data));
    }
    (sources, truncated)
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
    let mut facts = PboConfigFacts::default();
    let Some(data) = read_capped(path, PBO_SCAN_LIMIT) else {
        return facts;
    };
    let text = readable_strings(&data);

    facts.required_addons = required_addons(&text);
    facts.str_tokens = str_tokens(&text);
    facts.cba_keybind_names = cba_names_from_entries(path, &text);

    // `section` is the CfgMods or CfgPatches block the scan is inside;
    // `section_depth` is the brace depth at which that block opened.
    let mut section = Section::Unknown;
    let mut section_depth = 0usize;
    let mut pending_section = Section::Unknown;
    let mut depth = 0usize;

    for statement in statements(&text) {
        match statement.as_str() {
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

        if let Some(name) = class_name(&statement) {
            match name.to_ascii_lowercase().as_str() {
                "cfgmods" => pending_section = Section::CfgMods,
                "cfgpatches" => pending_section = Section::CfgPatches,
                _ if section == Section::CfgPatches
                    && depth == section_depth + 1
                    && facts.cfgpatches_class.is_none() =>
                {
                    facts.cfgpatches_class = Some(name.to_string());
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

    if let Some(rap) = rap_from_config_entries(path).or_else(|| parse_rap_config(&data)) {
        merge_rap_facts(&mut facts, rap);
    }

    facts
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

/// Categories from CBA keybind registrations, first the 4 MiB window
/// then every stored authored-text entry, because a mod's keybind code is
/// often near the end of a large PBO.
fn cba_names_from_entries(path: &Path, head: &str) -> Vec<String> {
    let mut names = cba_keybind_names(head);
    if !names.is_empty() {
        names.sort();
        names.dedup();
        return names;
    }

    let (sources, _) = pbo_stored_sources_capped(path);
    for (name, bytes) in sources {
        if !is_authored_text(&name) {
            continue;
        }
        names.extend(cba_keybind_names(&readable_strings(&bytes)));
    }
    names.sort();
    names.dedup();
    names
}

/// The first quoted string of a `CBA_fnc_addKeybind` array. The marker
/// closes the array, so the category is on the same printable line.
fn cba_keybind_names(text: &str) -> Vec<String> {
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

/// Printable runs of at least four bytes, one per line, so the field
/// scans can work line by line. Braces get their own line: they carry
/// the nesting the scan needs.
fn readable_strings(data: &[u8]) -> String {
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
    if run.len() >= 4 {
        text.push_str(&String::from_utf8_lossy(run));
        text.push('\n');
    }
    run.clear();
}

/// Split config text into class declarations, braces, statements and
/// terminators, so a one-line config parses like a formatted one.
fn statements(text: &str) -> Vec<String> {
    text.replace(';', "\n")
        .lines()
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .map(str::to_string)
        .collect()
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

/// Every `requiredAddons[]` entry, in declaration order. The scan
/// tolerates the array wrapping across lines.
fn required_addons(text: &str) -> Vec<String> {
    let mut addons: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("requiredAddons[]") {
        rest = &rest[index + "requiredAddons[]".len()..];
        let after = rest.trim_start();
        let Some(after) = after.strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let Some(after) = after.strip_prefix('{') else {
            continue;
        };
        let Some((body, after_close)) = after.split_once('}') else {
            continue;
        };
        for entry in body.split(',') {
            let addon: String = entry
                .chars()
                .filter(|c| !c.is_whitespace() && *c != '"')
                .collect();
            if !addon.is_empty() && !addons.contains(&addon) {
                addons.push(addon);
            }
        }
        rest = after_close;
    }
    addons
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
        if path.len() == 1
            && path[0].eq_ignore_ascii_case("CfgPatches")
            && self.facts.cfgpatches_class.is_none()
        {
            self.facts.cfgpatches_class = Some(name.to_string());
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

/// A stored `config.bin` at any depth in the PBO.
fn is_binary_config_entry(name: &str) -> bool {
    name.rsplit(['\\', '/'])
        .next()
        .map(|stem| stem.eq_ignore_ascii_case("config.bin"))
        .unwrap_or(false)
}

/// Parse the stored config entries of a PBO. A large pack puts its config
/// beyond the scan window, so the entry itself is read. The pack's own
/// config sits at the root; a nested one configures a single object.
fn rap_from_config_entries(path: &Path) -> Option<RapFacts> {
    use std::io::{Read, Seek, SeekFrom};

    let mut entries: Vec<PboEntry> = pbo_entries(path)
        .into_iter()
        .filter(|entry| {
            entry.mime == 0
                && u64::from(entry.data_size) <= PBO_CONFIG_ENTRY_LIMIT
                && is_binary_config_entry(&entry.name)
        })
        .collect();
    entries.sort_by_key(|entry| entry.name.contains(['\\', '/']) as u8);

    let mut file = fs::File::open(path).ok()?;
    for entry in entries.into_iter().take(PBO_MAX_CONFIG_ENTRIES) {
        let mut data = Vec::new();
        if file.seek(SeekFrom::Start(entry.offset)).is_err() {
            return None;
        }
        if file
            .by_ref()
            .take(u64::from(entry.data_size))
            .read_to_end(&mut data)
            .is_err()
        {
            continue;
        }
        if let Some(facts) = parse_rap_config(&data) {
            return Some(facts);
        }
    }
    None
}

#[cfg(test)]
mod tests {
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
    fn write_pbo_entries(path: &Path, entries: &[(&str, u32, &[u8])]) {
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
