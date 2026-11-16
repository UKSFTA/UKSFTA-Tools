//! Learned evidence pool: every human-readable string inside a PBO is
//! evidence, ranked by rarity and title shape.
//!
//! Two passes make rarity real. The first pass harvests strings from
//! every stored PBO entry and counts how many PBOs contain each phrase
//! and each token. The second pass scores with those counts. No mod, key
//! or author is special-cased: the only static list is structural
//! language and config vocabulary.
//!
//! A phrase loses score when it opens with interface vocabulary or when
//! a `displayName` assignment in the same PBO produced it. A mod title
//! therefore outranks an item label and a menu entry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::normalise::normalise;
use super::plan::CorpusStats;
use crate::pbo::pbo_stored_sources_capped;

/// Strings kept per PBO.
const MAX_STRINGS_PER_PBO: usize = 20_000;
/// Candidate fragments examined per PBO, before the shape filter.
const MAX_CANDIDATES_PER_PBO: usize = 100_000;
/// Strings kept for the whole corpus.
const MAX_POOL_STRINGS: usize = 200_000;
/// Distinct tokens kept for the whole corpus.
const MAX_POOL_TOKENS: usize = 200_000;
const MIN_RUN: usize = 4;
const MAX_RUN: usize = 80;
const MAX_WORDS: usize = 6;

/// Shape score of a single token that is ALL CAPS or brand-led.
const SINGLE_TOKEN_SHAPE: f64 = 0.3;
/// Shape score of a two-word phrase, before casing is counted.
const TWO_WORD_SHAPE: f64 = 0.45;
/// Shape score of a three-to-five-word phrase, before casing is counted.
const TITLE_WORD_SHAPE: f64 = 0.7;
/// Multiplier for a phrase that opens with interface vocabulary.
const UI_WORD_PENALTY: f64 = 0.45;
/// Multiplier for a phrase that a `displayName` assignment produced.
const ITEM_LABEL_PENALTY: f64 = 0.6;
/// Added to a phrase that contains an anchor token.
const ANCHOR_BONUS: f64 = 0.2;

/// Where a string sat in the entry, strongest context first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum EvidenceKind {
    Bare,
    Quoted,
    Assignment,
}

/// One harvested string.
///
/// `token_df` is the document frequency of the string's rarest token.
/// `pbo_df` is how many PBOs contain this phrase. `pbo` binds the string
/// to its source so the planner can rank one PBO at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceString {
    pub pbo: String,
    pub text: String,
    pub kind: EvidenceKind,
    pub token_df: usize,
    pub pbo_df: usize,
}

/// The corpus-wide pool. `strings` is ordered by PBO and then by rank.
///
/// The per-PBO set of `displayName` values rides in `corpus.df` under
/// [`label_key`]: a normalised token is alphanumeric, so a keyed label
/// can never collide with a token count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct EvidencePool {
    pub strings: Vec<EvidenceString>,
    pub corpus: CorpusStats,
    pub truncated: bool,
}

/// Separates the PBO from the phrase in a label key.
const LABEL_SEPARATOR: char = '\u{1}';

/// The key that records `text` as a `displayName` value of `pbo`.
///
/// The label set has no field of its own: `plan.rs` and `match.rs` build
/// a pool by struct literal, so a new field would break them.
fn label_key(pbo: &str, text: &str) -> String {
    format!("{LABEL_SEPARATOR}{pbo}{LABEL_SEPARATOR}{text}")
}

#[derive(Debug, Clone)]
struct RawString {
    pbo: String,
    text: String,
    kind: EvidenceKind,
    tokens: Vec<String>,
}

/// Harvest every PBO in `addons_dir`, then score with learned document
/// frequencies.
pub(crate) fn build_evidence_pool(addons_dir: &Path) -> EvidencePool {
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(addons_dir) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().map(|ext| ext == "pbo").unwrap_or(false))
            .collect(),
        Err(_) => Vec::new(),
    };
    paths.sort();

    let mut harvested: Vec<RawString> = Vec::new();
    let mut phrase_docs: BTreeMap<String, usize> = BTreeMap::new();
    let mut token_docs: BTreeMap<String, usize> = BTreeMap::new();
    let mut docs = 0usize;
    let mut truncated = false;

    'pbo: for path in paths {
        let pbo = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (sources, cut) = pbo_stored_sources_capped(&path);
        truncated |= cut;
        docs += 1;

        let (strings, labels) = harvest_sources(&pbo, &sources, &mut truncated);
        let mut seen_phrases = BTreeMap::new();
        let mut seen_tokens = BTreeMap::new();

        for item in strings {
            let key = normalise(&item.text).text;
            if key.is_empty() {
                continue;
            }
            if seen_phrases.insert(key.clone(), ()).is_none() {
                *phrase_docs.entry(key).or_insert(0) += 1;
            }
            for token in &item.tokens {
                if seen_tokens.insert(token.clone(), ()).is_none() {
                    if let Some(count) = token_docs.get_mut(token) {
                        *count += 1;
                    } else if token_docs.len() < MAX_POOL_TOKENS {
                        token_docs.insert(token.clone(), 1);
                    } else {
                        truncated = true;
                    }
                }
            }
            if harvested.len() >= MAX_POOL_STRINGS {
                truncated = true;
                break 'pbo;
            }
            harvested.push(item);
        }

        for label in labels {
            let key = label_key(&pbo, &label);
            if token_docs.contains_key(&key) {
                continue;
            }
            if token_docs.len() < MAX_POOL_TOKENS {
                token_docs.insert(key, 1);
            } else {
                truncated = true;
            }
        }
    }

    let strings: Vec<EvidenceString> = harvested
        .into_iter()
        .map(|item| {
            let key = normalise(&item.text).text;
            let pbo_df = phrase_docs.get(&key).copied().unwrap_or(1);
            let token_df = item
                .tokens
                .iter()
                .filter_map(|token| token_docs.get(token))
                .copied()
                .min()
                .unwrap_or(pbo_df);
            EvidenceString {
                pbo: item.pbo,
                text: item.text,
                kind: item.kind,
                token_df,
                pbo_df,
            }
        })
        .collect();

    let corpus = CorpusStats {
        df: token_docs.into_iter().collect(),
        docs,
    };
    let mut scored: Vec<(f64, EvidenceString)> = strings
        .into_iter()
        .map(|item| {
            let key = normalise(&item.text).text;
            let is_label = corpus.df.contains_key(&label_key(&item.pbo, &key));
            let score = evidence_score(&item, corpus.docs, is_label, false);
            (score, item)
        })
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| {
        a.pbo
            .cmp(&b.pbo)
            .then_with(|| score_order(*a_score, a, *b_score, b))
    });
    let strings = scored.into_iter().map(|(_, item)| item).collect();

    EvidencePool {
        strings,
        corpus,
        truncated,
    }
}

/// The best phrases in the whole pool, strongest first.
///
/// Only the evidence tests rank the whole pool; the planner ranks per PBO.
#[cfg(test)]
pub(crate) fn rank_phrases(pool: &EvidencePool, limit: usize) -> Vec<EvidenceString> {
    rank_items(pool, pool.strings.iter(), &[], limit)
}

/// The best phrases harvested from one PBO, strongest first.
pub(crate) fn rank_phrases_for(
    pool: &EvidencePool,
    pbo: &str,
    limit: usize,
) -> Vec<EvidenceString> {
    rank_phrases_for_with(pool, pbo, &[], limit)
}

/// The best phrases from one PBO, with anchors that lift a match.
///
/// An anchor is a class root, a prefix family or a category token. A
/// phrase that holds one names this addon, so it gains score over a
/// phrase that merely shares the PBO.
pub(crate) fn rank_phrases_for_with(
    pool: &EvidencePool,
    pbo: &str,
    anchors: &[String],
    limit: usize,
) -> Vec<EvidenceString> {
    rank_items(
        pool,
        pool.strings.iter().filter(|item| item.pbo == pbo),
        anchors,
        limit,
    )
}

fn rank_items<'a>(
    pool: &EvidencePool,
    items: impl Iterator<Item = &'a EvidenceString>,
    anchors: &[String],
    limit: usize,
) -> Vec<EvidenceString> {
    let anchors = anchor_tokens(anchors);
    let mut scored: Vec<(f64, &EvidenceString)> = items
        .map(|item| {
            let normalised = normalise(&item.text);
            let is_label = pool
                .corpus
                .df
                .contains_key(&label_key(&item.pbo, &normalised.text));
            let anchored = normalised
                .tokens
                .iter()
                .any(|token| anchors.contains(token));
            (
                evidence_score(item, pool.corpus.docs, is_label, anchored),
                item,
            )
        })
        .collect();
    scored.sort_by(|(a_score, a), (b_score, b)| score_order(*a_score, a, *b_score, b));
    dedupe(scored.into_iter().map(|(_, item)| item).collect(), limit)
}

/// Anchor tokens, normalised so `JAS_GPNVG18` and `jas gpnvg18` agree.
fn anchor_tokens(anchors: &[String]) -> BTreeSet<String> {
    anchors
        .iter()
        .flat_map(|anchor| normalise(anchor).tokens)
        .collect()
}

fn dedupe(ranked: Vec<&EvidenceString>, limit: usize) -> Vec<EvidenceString> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for item in ranked {
        if !seen.insert(item.text.to_lowercase()) {
            continue;
        }
        out.push(item.clone());
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// Score order, then a stable text order so a tie never moves.
fn score_order(
    a_score: f64,
    a: &EvidenceString,
    b_score: f64,
    b: &EvidenceString,
) -> std::cmp::Ordering {
    b_score
        .total_cmp(&a_score)
        .then_with(|| a.text.to_lowercase().cmp(&b.text.to_lowercase()))
        .then_with(|| a.text.cmp(&b.text))
        .then_with(|| a.pbo.cmp(&b.pbo))
}

/// Rarity first, then title shape, then where the string sat. Interface
/// vocabulary and item labels scale the result down. An anchor lifts it.
fn evidence_score(item: &EvidenceString, docs: usize, is_label: bool, anchored: bool) -> f64 {
    let rarity = if docs == 0 {
        0.0
    } else {
        (1.0 - item.pbo_df as f64 / docs as f64).max(0.0)
    };
    let context = match item.kind {
        EvidenceKind::Assignment => 1.0,
        EvidenceKind::Quoted => 0.7,
        EvidenceKind::Bare => 0.3,
    };
    let mut score = 0.55 * rarity + 0.30 * title_shape(&item.text) + 0.15 * context;
    if ui_phrase(&item.text) {
        score *= UI_WORD_PENALTY;
    }
    if is_label {
        score *= ITEM_LABEL_PENALTY;
    }
    if anchored {
        score += ANCHOR_BONUS;
    }
    score
}

/// A written title, not an item label or a menu entry.
///
/// Three to five words in Title Case or ALL CAPS score highest. Two
/// words score lower, because most two-word phrases name one item. A
/// single token scores only when it is ALL CAPS or brand-led, and then
/// weakly. Title Case outranks ALL CAPS: a shouted phrase is weaker
/// evidence than a written product name.
fn title_shape(text: &str) -> f64 {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || words.len() > 5 {
        return 0.0;
    }
    if words.len() == 1 {
        let word = words[0];
        return if is_all_caps(word) || is_brand_token(word) {
            SINGLE_TOKEN_SHAPE
        } else {
            0.0
        };
    }
    let mut shape = if words.len() >= 3 {
        TITLE_WORD_SHAPE
    } else {
        TWO_WORD_SHAPE
    };
    let all_caps = words.iter().all(|word| is_all_caps(word));
    let title_case = words.iter().all(|word| starts_uppercase(word));
    if all_caps {
        shape += 0.2;
    } else if title_case {
        shape += 0.3;
    }
    if words.iter().any(|word| is_brand_token(word)) {
        shape += 0.1;
    }
    shape.min(1.0)
}

/// Interface vocabulary: a phrase that opens with one of these words
/// names a control, not an addon.
const UI_FIRST_WORDS: &[&str] = &[
    "user", "enable", "disable", "toggle", "open", "close", "drop", "remove", "select", "set",
    "show", "hide", "add", "save", "load", "reset", "spawn", "teleport", "settings", "options",
    "menu", "action",
];

fn ui_phrase(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .map(str::to_lowercase)
        .is_some_and(|word| UI_FIRST_WORDS.contains(&word.as_str()))
}

fn starts_uppercase(word: &str) -> bool {
    word.chars()
        .next()
        .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
}

fn is_all_caps(word: &str) -> bool {
    word.chars().any(char::is_alphabetic)
        && word
            .chars()
            .filter(|c| c.is_alphabetic())
            .all(char::is_uppercase)
}

fn is_brand_token(word: &str) -> bool {
    word.chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(false)
        && word.chars().any(|c| c.is_ascii_alphabetic())
}

/// File-name extensions that carry authored text. Binary or container
/// entries are skipped: random printable runs in a model or texture are
/// not human-readable strings. The list is structural, not a mod signal.
const TEXT_EXTENSIONS: &[&str] = &[
    "cpp", "hpp", "sqf", "ext", "cfg", "h", "txt", "xml", "json", "md", "inc", "rpt",
];

fn textual_entry(name: &str) -> bool {
    name.rsplit('.')
        .next()
        .map(|extension| TEXT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn harvest_sources(
    pbo: &str,
    sources: &[(String, Vec<u8>)],
    truncated: &mut bool,
) -> (Vec<RawString>, BTreeSet<String>) {
    let mut unique: BTreeMap<String, RawString> = BTreeMap::new();
    let mut labels: Vec<String> = Vec::new();
    let mut candidates = 0usize;

    'sources: for (name, bytes) in sources {
        if !textual_entry(name) {
            continue;
        }
        let (texts, found) = candidates_from_entry(bytes);
        labels.extend(found);
        for (text, kind) in texts {
            if candidates >= MAX_CANDIDATES_PER_PBO || unique.len() >= MAX_STRINGS_PER_PBO {
                *truncated = true;
                break 'sources;
            }
            candidates += 1;
            insert_candidate(&mut unique, pbo, text, kind);
        }
    }

    // A stringtable resolves `$STR_` tokens to the text a person wrote.
    // Those values are evidence in their own right, and on an asset-only
    // PBO they are often the only human text it carries.
    let tokens = stringtable_tokens(sources);
    for text in tokens.values() {
        if unique.len() >= MAX_STRINGS_PER_PBO {
            *truncated = true;
            break;
        }
        insert_candidate(&mut unique, pbo, text.clone(), EvidenceKind::Quoted);
    }

    let labels = labels
        .iter()
        .filter_map(|label| label_phrase(label, &tokens))
        .collect();
    (unique.into_values().collect(), labels)
}

/// A `displayName` value as a phrase key, with a `$STR_` token resolved.
fn label_phrase(label: &str, tokens: &BTreeMap<String, String>) -> Option<String> {
    let resolved = match label.strip_prefix('$') {
        Some(key) => tokens.get(&format!("${}", key.to_ascii_uppercase()))?,
        None => label,
    };
    let normalised = normalise(resolved);
    if normalised.tokens.is_empty() || normalised.tokens.len() > MAX_WORDS {
        return None;
    }
    Some(normalised.text)
}

/// Add one candidate under its normalised text, keeping the strongest
/// kind seen.
fn insert_candidate(
    unique: &mut BTreeMap<String, RawString>,
    pbo: &str,
    text: String,
    kind: EvidenceKind,
) {
    if !keep_shape(&text) {
        return;
    }
    let normalised = normalise(&text);
    if normalised.tokens.is_empty() || normalised.tokens.len() > MAX_WORDS {
        return;
    }
    let key = normalised.text;
    match unique.get_mut(&key) {
        Some(existing) => {
            if kind > existing.kind {
                existing.kind = kind;
                existing.text = text;
            }
        }
        None => {
            unique.insert(
                key,
                RawString {
                    pbo: pbo.to_string(),
                    text,
                    kind,
                    tokens: normalised.tokens,
                },
            );
        }
    }
}

/// Stringtable key/value pairs from a PBO, keyed by `$STR_` token.
///
/// Two shapes occur: a CSV table of `key,value` rows and the XML
/// `<Key ID="...">` element with an `<Original>` child. A table's own
/// casing does not matter, so the token is upper case.
fn stringtable_tokens(sources: &[(String, Vec<u8>)]) -> BTreeMap<String, String> {
    let mut tokens: BTreeMap<String, String> = BTreeMap::new();
    for (name, bytes) in sources {
        let lower = name.to_ascii_lowercase();
        let text = entry_text(bytes);
        if lower.ends_with(".csv") {
            collect_csv_pairs(&text, &mut tokens);
        } else if lower.ends_with(".xml") {
            collect_xml_strings(&text, &mut tokens);
        }
    }
    tokens
}

/// Entry bytes as text. A UTF-8 byte-order mark is dropped so the first
/// key is still a key.
fn entry_text(bytes: &[u8]) -> String {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

/// Rows of `key,value`. The first two columns carry the pair; a row whose
/// key reads `KEY` is a header.
fn collect_csv_pairs(text: &str, tokens: &mut BTreeMap<String, String>) {
    for line in text.lines() {
        let columns = csv_columns(line);
        let (Some(key), Some(value)) = (columns.first(), columns.get(1)) else {
            continue;
        };
        if key.eq_ignore_ascii_case("key") {
            continue;
        }
        insert_token(tokens, key, value);
    }
}

/// The columns of one CSV row, with quoting honoured.
fn csv_columns(line: &str) -> Vec<String> {
    let mut columns = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for character in line.chars() {
        match character {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                columns.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    columns.push(current.trim().to_string());
    columns
}

/// `<Key ID="STR_X">` blocks whose `<Original>` child carries the text.
fn collect_xml_strings(text: &str, tokens: &mut BTreeMap<String, String>) {
    let mut rest = text;
    while let Some(index) = rest.find("<Key") {
        rest = &rest[index + "<Key".len()..];
        let Some(tag_end) = rest.find('>') else {
            return;
        };
        let attributes = &rest[..tag_end];
        let after_tag = &rest[tag_end + 1..];
        let Some(close) = after_tag.find("</Key>") else {
            return;
        };
        let body = &after_tag[..close];
        rest = &after_tag[close + "</Key>".len()..];
        let (Some(id), Some(value)) = (
            xml_attribute(attributes, "ID"),
            xml_element(body, "Original"),
        ) else {
            continue;
        };
        insert_token(tokens, &id, &value);
    }
}

/// The value of one XML attribute, matched case-insensitively.
fn xml_attribute(attributes: &str, name: &str) -> Option<String> {
    let lower = attributes.to_ascii_lowercase();
    let needle = name.to_ascii_lowercase();
    let mut from = 0;
    while let Some(index) = lower.get(from..)?.find(needle.as_str()) {
        let start = from + index;
        from = start + needle.len();
        let spaced = attributes[..start]
            .chars()
            .next_back()
            .map(char::is_whitespace)
            .unwrap_or(false);
        if !spaced {
            continue;
        }
        let value = attributes[from..].trim_start();
        let Some(value) = value.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let quote = value.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let inner = &value[1..];
        let end = inner.find(quote)?;
        return Some(inner[..end].trim().to_string());
    }
    None
}

/// The text of the first `<name>...</name>` child, trimmed.
fn xml_element(text: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    let value = text[start..end].trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Record one table pair under its `$STR_` token. The first value for a
/// token wins, so a table of several languages keeps its first column.
fn insert_token(tokens: &mut BTreeMap<String, String>, key: &str, value: &str) {
    let key = key.trim().trim_start_matches('$');
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return;
    }
    let token = format!("${}", key.to_ascii_uppercase());
    tokens.entry(token).or_insert_with(|| value.to_string());
}

fn candidates_from_entry(bytes: &[u8]) -> (Vec<(String, EvidenceKind)>, Vec<String>) {
    let mut out = Vec::new();
    let mut labels = Vec::new();
    let text = printable_text(bytes);
    for line in text.lines() {
        let line = line.trim();
        if line.len() < MIN_RUN {
            continue;
        }

        let mut rest = line;
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else {
                break;
            };
            let content = after[..close].trim();
            if (MIN_RUN..=MAX_RUN).contains(&content.len()) {
                let kind = if rest[..open].contains('=') {
                    EvidenceKind::Assignment
                } else {
                    EvidenceKind::Quoted
                };
                if is_display_name_assignment(&rest[..open]) {
                    labels.push(content.to_string());
                }
                out.push((content.to_string(), kind));
            }
            rest = &after[close + 1..];
        }

        for fragment in line.split([';', '{', '}', '[', ']', '(', ')', ',']) {
            let fragment = fragment.trim().trim_matches('"').trim();
            if fragment.len() < MIN_RUN {
                continue;
            }
            let display_name = is_display_name_assignment(fragment);
            let (candidate, kind) = match fragment.split_once('=') {
                Some((_, value)) => (
                    value.trim().trim_matches('"').trim(),
                    EvidenceKind::Assignment,
                ),
                None => (fragment, EvidenceKind::Bare),
            };
            if (MIN_RUN..=MAX_RUN).contains(&candidate.len()) {
                if display_name {
                    labels.push(candidate.to_string());
                }
                out.push((candidate.to_string(), kind));
            }
        }
    }
    (out, labels)
}

/// True when `statement` assigns to `displayName`.
fn is_display_name_assignment(statement: &str) -> bool {
    let Some((key, _)) = statement.split_once('=') else {
        return false;
    };
    key.trim()
        .trim_end_matches(['[', ']'])
        .rsplit(|c: char| c.is_whitespace() || c == '.')
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case("displayName"))
}

/// Printable ASCII runs. Every other byte becomes a line break.
fn printable_text(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for &byte in bytes {
        if (0x20..=0x7e).contains(&byte) {
            text.push(char::from(byte));
        } else {
            text.push('\n');
        }
    }
    text
}

/// Shape filter: a human-readable phrase, not a path, file name or
/// identifier.
fn keep_shape(text: &str) -> bool {
    let text = text.trim();
    let length = text.chars().count();
    if !(MIN_RUN..=MAX_RUN).contains(&length) {
        return false;
    }
    if !text.chars().any(char::is_alphabetic) {
        return false;
    }
    if text
        .chars()
        .any(|c| matches!(c, '\\' | '/' | ':' | '%' | '#' | '@' | '$'))
    {
        return false;
    }
    if is_file_name(text) {
        return false;
    }
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || words.len() > MAX_WORDS {
        return false;
    }
    if words.len() == 1 && words[0].contains('_') {
        return false;
    }
    true
}

fn is_file_name(text: &str) -> bool {
    let Some((stem, extension)) = text.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !text.contains(char::is_whitespace)
        && (1..=5).contains(&extension.chars().count())
        && extension.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::super::plan::CorpusStats;
    use super::*;
    use std::collections::HashMap;

    /// Build a PBO with the given stored entries.
    fn write_pbo_files(path: &Path, entries: &[(&str, &[u8])]) {
        let properties = b"prefix\0test_prefix\0\0";
        let mut bytes = Vec::new();
        bytes.push(0);
        bytes.extend(0x5665_7273u32.to_le_bytes()); // Vers
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend((properties.len() as u32).to_le_bytes());
        bytes.extend(properties);
        for (name, data) in entries {
            bytes.extend(name.as_bytes());
            bytes.push(0);
            bytes.extend(0u32.to_le_bytes()); // stored
            bytes.extend((data.len() as u32).to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            bytes.extend(0u32.to_le_bytes());
            bytes.extend((data.len() as u32).to_le_bytes());
        }
        bytes.extend([0u8; 21]);
        for (_, data) in entries {
            bytes.extend(*data);
        }
        std::fs::write(path, bytes).unwrap();
    }

    /// Build a PBO with one stored `config.cpp` entry.
    fn write_pbo(path: &Path, config: &[u8]) {
        write_pbo_files(path, &[("config.cpp", config)]);
    }

    fn addons_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn evidence_rejects_paths_and_identifiers() {
        let dir = addons_dir("uksfta-evidence-shape");
        let config = br#"model = "data\models\SingleBottle.p3d";
            texture = "data\icons\ade_icon.paa";
            [SINGLE_TOKEN_ID, PATH_LIKE_VALUE] call fnc;
            id = "Diving Mask";
        "#;
        write_pbo(&dir.join("mod.pbo"), config);

        let pool = build_evidence_pool(&dir);

        for item in &pool.strings {
            assert!(!item.text.contains('\\'), "path kept: {}", item.text);
            assert!(!item.text.contains(".p3d"), "file name kept: {}", item.text);
            assert!(!item.text.contains(".paa"), "file name kept: {}", item.text);
            assert!(
                !(item.text.split_whitespace().count() == 1 && item.text.contains('_')),
                "identifier kept: {}",
                item.text
            );
        }
        assert!(pool.strings.iter().any(|item| item.text == "Diving Mask"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn evidence_finds_title_like_strings() {
        let dir = addons_dir("uksfta-evidence-title");
        let config = br#"["Advanced Diving Environment","adeVarDiveComputerVisible", "Toggle Dive Computer", {execVM "ade_core\scripts\run.sqf";}, {}, [0x3e, [true,false,false]]] call fnc_addKeybind;"#;
        write_pbo(&dir.join("ade_core.pbo"), config);

        let pool = build_evidence_pool(&dir);
        let ranked = rank_phrases(&pool, 10);

        assert!(
            ranked
                .iter()
                .any(|item| item.text == "Advanced Diving Environment"),
            "ranked: {:?}",
            ranked.iter().map(|item| &item.text).collect::<Vec<_>>()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn evidence_prefers_rare_phrase_over_common_phrase() {
        let dir = addons_dir("uksfta-evidence-rarity");
        write_pbo(
            &dir.join("one.pbo"),
            b"[\"Common Mod Name\", \"Unique Signature Title\"]",
        );
        write_pbo(&dir.join("two.pbo"), b"[\"Common Mod Name\"]");

        let pool = build_evidence_pool(&dir);
        let ranked = rank_phrases(&pool, 10);
        let position = |text: &str| ranked.iter().position(|item| item.text == text);

        let rare = position("Unique Signature Title").expect("rare phrase present");
        let common = position("Common Mod Name").expect("common phrase present");
        assert!(rare < common, "rare={rare} common={common}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn evidence_pool_is_deterministic() {
        let dir = addons_dir("uksfta-evidence-deterministic");
        write_pbo(
            &dir.join("a.pbo"),
            b"[\"Alpha Beta Title\", \"Gamma Delta Title\"]",
        );
        write_pbo(&dir.join("b.pbo"), b"[\"Alpha Beta Title\"]");

        let first = build_evidence_pool(&dir);
        let second = build_evidence_pool(&dir);

        assert_eq!(first, second);
        assert_eq!(rank_phrases(&first, 10), rank_phrases(&second, 10));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn evidence_counts_phrase_documents_once_per_pbo() {
        let item = |pbo: &str, pbo_df: usize| EvidenceString {
            pbo: pbo.to_string(),
            text: "Diving Mask".to_string(),
            kind: EvidenceKind::Quoted,
            token_df: 1,
            pbo_df,
        };
        let docs = 4;
        let rare = evidence_score(&item("a", 1), docs, false, false);
        let common = evidence_score(&item("a", 4), docs, false, false);

        assert!(rare > common);
        assert_eq!(
            evidence_score(&item("a", 4), docs, false, false),
            0.30 * title_shape("Diving Mask") + 0.15 * 0.7
        );
    }

    #[test]
    fn rank_phrases_limits_and_dedupes_case_insensitively() {
        let pool = EvidencePool {
            strings: vec![
                EvidenceString {
                    pbo: "a".to_string(),
                    text: "Same Title".to_string(),
                    kind: EvidenceKind::Assignment,
                    token_df: 1,
                    pbo_df: 1,
                },
                EvidenceString {
                    pbo: "a".to_string(),
                    text: "same title".to_string(),
                    kind: EvidenceKind::Bare,
                    token_df: 1,
                    pbo_df: 1,
                },
            ],
            corpus: CorpusStats {
                df: HashMap::new(),
                docs: 2,
            },
            truncated: false,
        };

        let ranked = rank_phrases(&pool, 5);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].text, "Same Title");
    }

    #[test]
    fn stringtable_links_str_token_to_value() {
        let dir = addons_dir("uksfta-evidence-stringtable-link");
        write_pbo_files(
            &dir.join("mod.pbo"),
            &[
                ("config.cpp", br#"author = "$STR_MYMOD_AUTHOR";"#),
                (
                    "stringtable.csv",
                    b"Key,English\nSTR_MYMOD_AUTHOR,Advanced Diving Systems\n",
                ),
            ],
        );

        let (sources, _) = pbo_stored_sources_capped(&dir.join("mod.pbo"));
        let tokens = stringtable_tokens(&sources);
        assert_eq!(
            tokens.get("$STR_MYMOD_AUTHOR").map(String::as_str),
            Some("Advanced Diving Systems")
        );

        let pool = build_evidence_pool(&dir);
        assert!(pool.strings.iter().any(
            |item| item.text == "Advanced Diving Systems" && item.kind == EvidenceKind::Quoted
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stringtable_parses_xml_original() {
        let sources = vec![(
            "stringtable.xml".to_string(),
            br#"<Project name="Mod"><Package name="Core"><Key ID="STR_MYMOD_NAME"><Original>BettIR Device</Original><English>BettIR Device</English></Key></Package></Project>"#.to_vec(),
        )];

        let tokens = stringtable_tokens(&sources);

        assert_eq!(
            tokens.get("$STR_MYMOD_NAME").map(String::as_str),
            Some("BettIR Device")
        );
    }

    #[test]
    fn stringtable_ignores_header_row() {
        let sources = vec![(
            "stringtable.csv".to_string(),
            b"Key,English\nSTR_MYMOD_NAME,Advanced Diving Systems\n".to_vec(),
        )];

        let tokens = stringtable_tokens(&sources);

        assert!(!tokens.contains_key("$KEY"));
        assert_eq!(
            tokens.get("$STR_MYMOD_NAME").map(String::as_str),
            Some("Advanced Diving Systems")
        );
    }

    #[test]
    fn ui_imperative_is_penalised() {
        let dir = addons_dir("uksfta-evidence-ui-phrase");
        write_pbo(
            &dir.join("mod.pbo"),
            br#"["User Actions", "Advanced Diving Environment"] call fnc_actions;"#,
        );

        let pool = build_evidence_pool(&dir);
        let ranked = rank_phrases_for(&pool, "mod.pbo", 10);
        let position = |text: &str| {
            ranked
                .iter()
                .position(|item| item.text == text)
                .unwrap_or_else(|| panic!("{} missing from {ranked:?}", text))
        };

        assert!(position("Advanced Diving Environment") < position("User Actions"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn three_word_title_outranks_item_label() {
        let dir = addons_dir("uksfta-evidence-item-label");
        write_pbo(
            &dir.join("mod.pbo"),
            br#"
                class CfgWeapons {
                    class ade_diving_mask {
                        displayName = "Diving Mask";
                    };
                };
                ["Advanced Diving Environment"] call fnc_titles;
            "#,
        );

        let pool = build_evidence_pool(&dir);
        let ranked = rank_phrases_for(&pool, "mod.pbo", 10);
        let position = |text: &str| {
            ranked
                .iter()
                .position(|item| item.text == text)
                .unwrap_or_else(|| panic!("{} missing from {ranked:?}", text))
        };

        assert!(position("Advanced Diving Environment") < position("Diving Mask"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn anchor_token_boosts_phrase() {
        let dir = addons_dir("uksfta-evidence-anchor");
        write_pbo(
            &dir.join("mod.pbo"),
            br#"["Alpha Beta Gamma", "Vulcan Scout Rifle"] call fnc_titles;"#,
        );

        let pool = build_evidence_pool(&dir);
        let plain = rank_phrases_for(&pool, "mod.pbo", 10);
        let anchored = rank_phrases_for_with(&pool, "mod.pbo", &["Vulcan".to_string()], 10);

        assert_eq!(plain[0].text, "Alpha Beta Gamma");
        assert_eq!(anchored[0].text, "Vulcan Scout Rifle");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn single_token_fragment_is_penalised() {
        let dir = addons_dir("uksfta-evidence-single-token");
        write_pbo(
            &dir.join("mod.pbo"),
            br#"["Environment", "Advanced Diving Environment"] call fnc_titles;"#,
        );

        let pool = build_evidence_pool(&dir);
        let ranked = rank_phrases_for(&pool, "mod.pbo", 10);
        let position = |text: &str| {
            ranked
                .iter()
                .position(|item| item.text == text)
                .unwrap_or_else(|| panic!("{} missing from {ranked:?}", text))
        };

        assert!(position("Advanced Diving Environment") < position("Environment"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
