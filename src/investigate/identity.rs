//! Ordered identity phrases for one PBO.
//!
//! One phrase per evidence source, strongest first. The order is data:
//! [`PhraseSource`] derives `Ord` in declaration order, so sorting by
//! source and deduplicating keeps the strongest text for each string.
//! `requiredAddons` is corroboration only and never becomes a phrase.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::plan::is_placeholder;
use crate::pbo::{pbo_fingerprint, pbo_prefix, PboConfigFacts, PboContent};

/// Evidence strength, strongest first. The declaration order is the
/// sort key, so no weight table can drift from the documented order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum PhraseSource {
    CfgModsName,
    /// The category shown in CBA addon options. Modders usually put the
    /// mod title or the Workshop name here.
    CbaOptionName,
    WorkshopUrl,
    TagAuthor,
    CfgPatchesClass,
    DisplayName,
    PrefixFamily,
    StrToken,
    Stem,
    /// A ranked string from the learned evidence pool.
    Dynamic,
}

/// One search phrase and the evidence it came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IdentityPhrase {
    pub text: String,
    pub source: PhraseSource,
    pub workshop_id: Option<String>,
}

/// Everything the planner and the scorer need to know about one PBO.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IdentityEvidence {
    pub pbo: String,
    pub fingerprint: String,
    pub phrases: Vec<IdentityPhrase>,
    pub cfgmods_author: Option<String>,
    pub cfgpatches_author: Option<String>,
    pub required_addons: Vec<String>,
    pub prefix: Option<String>,
    pub stem: String,
}

/// Derive the ordered phrases from a PBO already read. The online path
/// passes the same read to the evidence harvester.
pub(crate) fn extract_identity_from(content: &PboContent) -> IdentityEvidence {
    let path = content.path.as_path();
    let pbo = content.name.clone();
    let prefix = pbo_prefix(path);
    let facts = content.config_facts();
    let phrases = identity_phrases(&pbo, prefix.as_deref(), &facts);
    let stem = Path::new(&pbo)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();

    IdentityEvidence {
        fingerprint: pbo_fingerprint(path).unwrap_or_default(),
        phrases,
        cfgmods_author: facts.cfgmods_author,
        cfgpatches_author: facts.cfgpatches_author,
        required_addons: facts.required_addons,
        prefix,
        stem,
        pbo,
    }
}

/// Build the phrase list for one PBO, strongest first.
///
/// Placeholders, pure numbers and texts shorter than three characters
/// are dropped. The list is sorted by source and deduplicated
/// case-insensitively, keeping the strongest source's text.
pub(crate) fn identity_phrases(
    pbo_name: &str,
    prefix: Option<&str>,
    facts: &PboConfigFacts,
) -> Vec<IdentityPhrase> {
    let mut phrases = Vec::new();

    if let Some(text) = facts.cfgmods_name.as_deref().and_then(useable) {
        phrases.push(IdentityPhrase {
            text,
            source: PhraseSource::CfgModsName,
            workshop_id: None,
        });
    }

    for name in &facts.cba_keybind_names {
        if let Some(text) = useable(name) {
            phrases.push(IdentityPhrase {
                text,
                source: PhraseSource::CbaOptionName,
                workshop_id: None,
            });
        }
    }

    if let Some(url) = facts.cfgpatches_url.as_deref() {
        if let Some(id) = workshop_id_from_url(url) {
            phrases.push(IdentityPhrase {
                text: url.trim().to_string(),
                source: PhraseSource::WorkshopUrl,
                workshop_id: Some(id),
            });
        }
    }

    if let Some(text) = bracket_tag(facts.cfgmods_author.as_deref())
        .or_else(|| bracket_tag(facts.cfgpatches_author.as_deref()))
    {
        phrases.push(IdentityPhrase {
            text,
            source: PhraseSource::TagAuthor,
            workshop_id: None,
        });
    }

    if let Some(text) = facts.cfgpatches_class.as_deref().and_then(useable) {
        phrases.push(IdentityPhrase {
            text,
            source: PhraseSource::CfgPatchesClass,
            workshop_id: None,
        });
    }

    for display in &facts.display_names {
        if let Some(text) = display_phrase(display) {
            phrases.push(IdentityPhrase {
                text,
                source: PhraseSource::DisplayName,
                workshop_id: None,
            });
        }
    }

    if let Some(text) = prefix.and_then(useable) {
        phrases.push(IdentityPhrase {
            text,
            source: PhraseSource::PrefixFamily,
            workshop_id: None,
        });
    }

    for token in &facts.str_tokens {
        if let Some(text) = useable(token) {
            phrases.push(IdentityPhrase {
                text,
                source: PhraseSource::StrToken,
                workshop_id: None,
            });
        }
    }

    let stem = Path::new(pbo_name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| pbo_name.to_string());
    if let Some(text) = useable(&stem) {
        phrases.push(IdentityPhrase {
            text,
            source: PhraseSource::Stem,
            workshop_id: None,
        });
    }

    // Stable sort keeps the source's own order (the first displayName
    // wins a tie), then dedupe keeps the strongest source's text.
    phrases.sort_by_key(|phrase| phrase.source);
    let mut seen = HashSet::new();
    phrases.retain(|phrase| seen.insert(phrase.text.to_lowercase()));
    phrases
}

/// The Workshop id carried by a `CfgPatches` url, if any.
pub(crate) fn workshop_id_from_url(url: &str) -> Option<String> {
    crate::util::extract_id(url)
}

fn useable(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.chars().count() < 3 {
        return None;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if is_placeholder(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

/// An author is evidence only when it is bracketed: `[ADE]` -> `ADE`.
fn bracket_tag(author: Option<&str>) -> Option<String> {
    let author = author?;
    let open = author.find('[')?;
    let close = author[open + 1..].find(']')? + open + 1;
    useable(&author[open + 1..close])
}

/// Drop a trailing bracket tag from a display name:
/// `Diving Mask (ADE)` -> `Diving Mask`. A parenthetical with spaces is
/// item detail, not a tag, and is kept.
fn display_phrase(display: &str) -> Option<String> {
    let trimmed = display.trim();
    let without_tag = match (trimmed.rfind('('), trimmed.ends_with(')')) {
        (Some(open), true) => {
            let content = &trimmed[open + 1..trimmed.len() - 1];
            if !content.contains(char::is_whitespace) && content.chars().count() <= 16 {
                trimmed[..open].trim_end()
            } else {
                trimmed
            }
        }
        _ => trimmed,
    };
    useable(without_tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn facts() -> PboConfigFacts {
        PboConfigFacts::default()
    }

    fn sources(phrases: &[IdentityPhrase]) -> Vec<PhraseSource> {
        phrases.iter().map(|phrase| phrase.source).collect()
    }

    #[test]
    fn identity_prefers_cfgmods_name_over_prefix() {
        let mut facts = facts();
        facts.cfgmods_name = Some("BettIR Core".to_string());

        let phrases = identity_phrases("BettIR_Core.pbo", Some("BettIR_Core"), &facts);

        assert_eq!(phrases[0].text, "BettIR Core");
        assert_eq!(phrases[0].source, PhraseSource::CfgModsName);
    }

    #[test]
    fn identity_uses_cfgpatches_url_workshop_id() {
        let mut facts = facts();
        facts.cfgpatches_url =
            Some("https://steamcommunity.com/sharedfiles/filedetails/?id=1340701737".to_string());

        let phrases = identity_phrases("example.pbo", None, &facts);
        let url = phrases
            .iter()
            .find(|phrase| phrase.source == PhraseSource::WorkshopUrl)
            .expect("workshop url phrase");

        assert_eq!(url.workshop_id.as_deref(), Some("1340701737"));
    }

    #[test]
    fn identity_extracts_bracket_author_tag() {
        let mut facts = facts();
        facts.cfgpatches_author = Some("[ADE]".to_string());

        let phrases = identity_phrases("ade_core.pbo", Some("ade_core"), &facts);
        let tag = phrases
            .iter()
            .find(|phrase| phrase.source == PhraseSource::TagAuthor)
            .expect("author tag phrase");

        assert_eq!(tag.text, "ADE");
    }

    #[test]
    fn identity_uses_display_name_phrase_when_no_mod_name() {
        let mut facts = facts();
        facts.display_names = vec!["Diving Mask (ADE)".to_string()];

        let phrases = identity_phrases("ade_core.pbo", Some("ade_core"), &facts);
        let display = phrases
            .iter()
            .find(|phrase| phrase.source == PhraseSource::DisplayName)
            .expect("display name phrase");

        assert_eq!(display.text, "Diving Mask");
    }

    #[test]
    fn identity_drops_required_addons_from_phrases() {
        let dir = std::env::temp_dir().join("uksfta-identity-wave-b-required-addons");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ade_core.pbo");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(
            br#"class CfgPatches { class ade_core { requiredAddons[] = {"Extended_EventHandlers","ace_common"}; author = "[ADE]"; displayName = "Diving Mask (ADE)"; }; };"#,
        )
        .unwrap();

        let evidence = extract_identity_from(&PboContent::read(&path));

        assert_eq!(
            evidence.required_addons,
            ["Extended_EventHandlers", "ace_common"]
        );
        assert!(evidence.phrases.iter().all(|phrase| {
            let text = phrase.text.to_lowercase();
            !text.contains("extended") && !text.contains("ace_common")
        }));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn identity_orders_phrases_by_strength() {
        let mut facts = facts();
        facts.cfgmods_name = Some("BettIR Core".to_string());
        facts.cfgpatches_url = Some("https://example.invalid/?id=1340701737".to_string());
        facts.cfgpatches_author = Some("[BIR]".to_string());
        facts.cfgpatches_class = Some("BettIR_Patches".to_string());
        facts.display_names = vec!["BettIR Device".to_string()];

        let phrases = identity_phrases("BettIR_Core.pbo", Some("BettIR_Addons"), &facts);

        assert_eq!(
            sources(&phrases),
            [
                PhraseSource::CfgModsName,
                PhraseSource::WorkshopUrl,
                PhraseSource::TagAuthor,
                PhraseSource::CfgPatchesClass,
                PhraseSource::DisplayName,
                PhraseSource::PrefixFamily,
                PhraseSource::Stem,
            ]
        );
        assert!(phrases
            .windows(2)
            .all(|pair| pair[0].source < pair[1].source));
    }

    #[test]
    fn identity_never_emits_str_placeholder() {
        let mut facts = facts();
        facts.str_tokens = vec!["$STR_BettIR_Name".to_string(), "$STR_DEVICE".to_string()];

        let phrases = identity_phrases("BettIR_Core.pbo", Some("BettIR_Core"), &facts);

        assert!(phrases
            .iter()
            .all(|phrase| !phrase.text.starts_with("$STR_")));
        assert!(phrases
            .iter()
            .all(|phrase| phrase.source != PhraseSource::StrToken));
    }

    #[test]
    fn identity_dedupes_case_insensitively_keeping_strongest() {
        let mut facts = facts();
        facts.cfgmods_name = Some("BettIR Core".to_string());

        let phrases = identity_phrases("bettir core.pbo", Some("bettir core"), &facts);

        assert_eq!(phrases.len(), 1);
        assert_eq!(phrases[0].source, PhraseSource::CfgModsName);
    }

    #[test]
    fn identity_drops_config_macro_display_names() {
        let mut facts = facts();
        facts.display_names = vec![
            "CSTRING".to_string(),
            "CSTRING(SatchelCharge_Remote_Mag_Throwable)".to_string(),
        ];

        let phrases = identity_phrases("ace_grenades.pbo", Some("ace_grenades"), &facts);

        assert!(phrases
            .iter()
            .all(|phrase| phrase.source != PhraseSource::DisplayName));
    }

    #[test]
    fn identity_drops_short_and_numeric_phrases() {
        let mut facts = facts();
        facts.cfgpatches_class = Some("42".to_string());
        facts.display_names = vec!["AB".to_string()];

        let phrases = identity_phrases("ab.pbo", None, &facts);

        assert!(phrases.is_empty());
    }
}

#[cfg(test)]
mod cba_identity_tests {
    use super::*;
    use crate::pbo::PboConfigFacts;

    #[test]
    fn identity_uses_cba_keybind_category() {
        let facts = PboConfigFacts {
            cba_keybind_names: vec!["Advanced Diving Environment".to_string()],
            ..Default::default()
        };

        let phrases = identity_phrases("ade_core.pbo", Some("ade_core"), &facts);

        assert!(phrases.iter().any(|phrase| {
            phrase.text == "Advanced Diving Environment"
                && phrase.source == PhraseSource::CbaOptionName
        }));
    }
}
