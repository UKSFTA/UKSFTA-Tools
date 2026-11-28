//! Query planning: at most three searches, most specific first.
//!
//! A plan is built from the ordered identity phrases. A query survives
//! only when its one anchor token is distinctive: not a placeholder, not
//! a generic word, at least four characters, not numeric, rare in the
//! local PBO corpus, and not shared by a group of three or more PBOs.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::evidence::{rank_phrases_for, EvidencePool};
use super::identity::{IdentityEvidence, IdentityPhrase, PhraseSource};
use super::normalise::normalise;

/// Queries planned per cluster before the evidence is treated as too
/// weak. Page two adds one more request to the best query.
const MAX_QUERIES: usize = 3;

/// An anchor above this document frequency names a family, not an addon.
const MAX_CORPUS_DF: f64 = 0.10;

/// Phrases examined on the learned path.
const MAX_DYNAMIC_PHRASES: usize = 40;

/// Why a query was planned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryKind {
    IdentityName,
    TagPlusHead,
    ClassRootPlusToken,
}

/// One search. `anchor` is the single token the title must contain
/// before the match score can rise above 0.40.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedQuery {
    pub text: String,
    pub kind: QueryKind,
    pub source: PhraseSource,
    pub anchor: String,
}

/// The ordered queries for one PBO.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct QueryPlan {
    pub queries: Vec<PlannedQuery>,
}

/// Token document frequencies across the local PBO corpus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CorpusStats {
    pub df: HashMap<String, usize>,
    pub docs: usize,
}

/// Inputs that are not part of one PBO's evidence.
pub(crate) struct PlannerContext<'a> {
    pub corpus: &'a CorpusStats,
    /// Tokens present in more than 60% of a group of three or more
    /// PBOs. The caller computes the share; the planner only rejects.
    pub group_terms: &'a HashSet<String>,
    /// Learned evidence for the dynamic path. `None` keeps the planner
    /// on typed seeds only.
    pub pool: Option<&'a EvidencePool>,
}

/// Count each normalised PBO name token once per document, over every
/// directory given.
///
/// One pack teaches rarity for that pack alone. The Workshop caches hold
/// the corpus a real name is rare in, so the planner reads all of them.
pub(crate) fn build_corpus_stats_many(dirs: &[PathBuf]) -> CorpusStats {
    let mut corpus = CorpusStats::default();
    for dir in dirs {
        count_documents(dir, &mut corpus);
    }
    corpus
}

/// Count each normalised PBO name token once per document.
// The online path builds from every cache; this wrapper serves the tests
// that check the counting against one directory.
#[cfg(test)]
pub(crate) fn build_corpus_stats(addons_dir: &Path) -> CorpusStats {
    build_corpus_stats_many(&[addons_dir.to_path_buf()])
}

fn count_documents(dir: &Path, corpus: &mut CorpusStats) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().map(|ext| ext != "pbo").unwrap_or(true) {
            continue;
        }
        corpus.docs += 1;
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut tokens = normalise(&stem).tokens;
        tokens.sort();
        tokens.dedup();
        for token in tokens {
            *corpus.df.entry(token).or_insert(0) += 1;
        }
    }
}

/// Build the query plan for one PBO.
pub(crate) fn plan_queries(evidence: &IdentityEvidence, ctx: &PlannerContext) -> QueryPlan {
    if let Some(url) = evidence
        .phrases
        .iter()
        .find(|phrase| phrase.source == PhraseSource::WorkshopUrl)
    {
        if let Some(id) = url.workshop_id.as_deref() {
            return QueryPlan {
                queries: vec![PlannedQuery {
                    text: url.text.clone(),
                    kind: QueryKind::IdentityName,
                    source: PhraseSource::WorkshopUrl,
                    anchor: id.to_string(),
                }],
            };
        }
    }

    let mut plan = QueryPlan::default();
    let tag = phrase_text(evidence, PhraseSource::TagAuthor);
    let display = strongest_display(evidence);

    // Q1: the mod's own name, else the strongest learned phrase, else the
    // strongest display name. A display name is item-level evidence: it
    // names one item, so it must not outrank a phrase the corpus ranks as
    // the mod's own name.
    let mut learned_first = false;
    if let Some(text) = phrase_text(evidence, PhraseSource::CfgModsName) {
        push_query(
            &mut plan,
            text,
            QueryKind::IdentityName,
            PhraseSource::CfgModsName,
            ctx,
        );
    } else if let Some(text) = phrase_text(evidence, PhraseSource::CbaOptionName) {
        push_query(
            &mut plan,
            text,
            QueryKind::IdentityName,
            PhraseSource::CbaOptionName,
            ctx,
        );
        learned_first = true;
    } else if push_top_dynamic(evidence, ctx, &mut plan) {
        learned_first = true;
    } else if let Some(phrase) = display {
        push_query(
            &mut plan,
            phrase.text.clone(),
            QueryKind::IdentityName,
            PhraseSource::DisplayName,
            ctx,
        );
    }

    // The displaced display name keeps the next free slot, so it still
    // wins where the item name is the mod name.
    if learned_first {
        if let Some(phrase) = display {
            push_query(
                &mut plan,
                phrase.text.clone(),
                QueryKind::IdentityName,
                PhraseSource::DisplayName,
                ctx,
            );
        }
    }

    // Author tag plus the display name's distinctive modifier, else the
    // prefix family plus its head noun.
    if let Some(tag) = tag {
        if let Some(second) = display
            .and_then(|phrase| modifier_token(&phrase.text))
            .or_else(|| evidence.prefix.as_deref().and_then(head_noun))
            .or_else(|| head_noun(&evidence.stem))
        {
            push_query(
                &mut plan,
                format!("{tag} {second}"),
                QueryKind::TagPlusHead,
                PhraseSource::TagAuthor,
                ctx,
            );
        }
    } else if let Some((text, source)) = prefix_query(evidence) {
        push_query(&mut plan, text, QueryKind::TagPlusHead, source, ctx);
    }

    // Class root plus the second class token.
    if let Some(class) = phrase_text(evidence, PhraseSource::CfgPatchesClass) {
        if let Some((text, anchor)) = class_query(&class) {
            push_anchored(
                &mut plan,
                text,
                QueryKind::ClassRootPlusToken,
                PhraseSource::CfgPatchesClass,
                anchor,
                ctx,
            );
        }
    }

    // No typed seed survived: rank the learned strings for this PBO.
    if plan.queries.is_empty() {
        plan_dynamic(evidence, ctx, &mut plan);
    }

    plan
}

/// Learned fallback, strongest first: the top ranked phrase, then the
/// top phrase containing the rarest token, then the two rarest tokens.
fn plan_dynamic(evidence: &IdentityEvidence, ctx: &PlannerContext, plan: &mut QueryPlan) {
    if push_top_dynamic(evidence, ctx, plan) {
        return;
    }
    let Some(pool) = ctx.pool else {
        return;
    };
    let phrases = rank_phrases_for(pool, &evidence.pbo, MAX_DYNAMIC_PHRASES);
    if phrases.is_empty() {
        return;
    }

    let Some(rarest) = rarest_token(&phrases, ctx) else {
        return;
    };
    if let Some(phrase) = phrases.iter().find(|phrase| {
        normalise(&phrase.text)
            .tokens
            .iter()
            .any(|token| token == &rarest)
    }) {
        push_query(
            plan,
            phrase.text.clone(),
            QueryKind::IdentityName,
            PhraseSource::Dynamic,
            ctx,
        );
    }
    if !plan.queries.is_empty() {
        return;
    }

    let tokens = rarest_tokens(&phrases, ctx, 2);
    if tokens.len() == 2 {
        push_query(
            plan,
            tokens.join(" "),
            QueryKind::IdentityName,
            PhraseSource::Dynamic,
            ctx,
        );
    }
}

/// Push the strongest learned phrase whose anchor is distinctive.
///
/// Returns `true` when a phrase was accepted.
fn push_top_dynamic(
    evidence: &IdentityEvidence,
    ctx: &PlannerContext,
    plan: &mut QueryPlan,
) -> bool {
    let Some(pool) = ctx.pool else {
        return false;
    };
    for phrase in rank_phrases_for(pool, &evidence.pbo, MAX_DYNAMIC_PHRASES) {
        let Some(anchor) = strongest_rare_token(&phrase.text, ctx) else {
            continue;
        };
        let before = plan.queries.len();
        push_anchored(
            plan,
            phrase.text.clone(),
            QueryKind::IdentityName,
            PhraseSource::Dynamic,
            anchor,
            ctx,
        );
        if plan.queries.len() > before {
            return true;
        }
    }
    false
}

/// The most distinctive usable token of a phrase: lowest document
/// frequency, then longest, then alphabetical.
fn strongest_rare_token(text: &str, ctx: &PlannerContext) -> Option<String> {
    normalise(text)
        .tokens
        .into_iter()
        .filter(|token| is_usable_anchor_with(token, ctx))
        .min_by(|a, b| rare_cmp(a, b, ctx))
}

fn rarest_token(
    phrases: &[super::evidence::EvidenceString],
    ctx: &PlannerContext,
) -> Option<String> {
    phrases
        .iter()
        .flat_map(|phrase| normalise(&phrase.text).tokens)
        .filter(|token| is_usable_anchor_with(token, ctx))
        .min_by(|a, b| rare_cmp(a, b, ctx))
}

fn rarest_tokens(
    phrases: &[super::evidence::EvidenceString],
    ctx: &PlannerContext,
    limit: usize,
) -> Vec<String> {
    let mut tokens: Vec<String> = phrases
        .iter()
        .flat_map(|phrase| normalise(&phrase.text).tokens)
        .filter(|token| is_usable_anchor_with(token, ctx))
        .collect();
    tokens.sort_by(|a, b| rare_cmp(a, b, ctx));
    tokens.dedup();
    tokens.truncate(limit);
    tokens
}

fn rare_cmp(a: &str, b: &str, ctx: &PlannerContext) -> std::cmp::Ordering {
    document_frequency(a, ctx)
        .cmp(&document_frequency(b, ctx))
        .then_with(|| b.chars().count().cmp(&a.chars().count()))
        .then_with(|| a.cmp(b))
}

fn document_frequency(token: &str, ctx: &PlannerContext) -> usize {
    ctx.corpus.df.get(token).copied().unwrap_or(0)
}

/// A word that names a whole mod family, not one addon.
///
/// This list is a seed heuristic for the typed path (head nouns,
/// display-name choice). Query rejection uses learned statistics:
/// [`is_structural_stop`], corpus document frequency and the group
/// share. It names no mod or product.
pub(crate) fn is_generic_word(word: &str) -> bool {
    const GENERIC: &[&str] = &[
        "mod",
        "core",
        "gear",
        "pack",
        "weapon",
        "uniform",
        "addon",
        "extended",
        "data",
        "main",
        "common",
        "base",
        "fix",
        "patch",
        "config",
        "vehicles",
        "units",
        "equipment",
        "misc",
        "sound",
        "ui",
        "nato",
        "nvg",
        "hud",
    ];
    let word = word.to_lowercase();
    GENERIC.contains(&word.as_str())
}

/// Language and config vocabulary only. These tokens describe the file
/// format, not an addon. No mod or product name belongs here.
pub(crate) fn is_structural_stop(word: &str) -> bool {
    const STOP: &[&str] = &[
        "class",
        "config",
        "displayname",
        "requiredaddons",
        "author",
        "url",
        "name",
        "value",
        "string",
        "text",
        "true",
        "false",
        "null",
        "the",
        "and",
        "for",
        "with",
        "from",
        "this",
        "that",
    ];
    STOP.contains(&word)
}

/// Learned rejection: a token names a family when the corpus says so.
fn is_learned_generic(token: &str, ctx: &PlannerContext) -> bool {
    if is_structural_stop(token) || ctx.group_terms.contains(token) {
        return true;
    }
    if ctx.corpus.docs == 0 {
        return false;
    }
    document_frequency(token, ctx) as f64 / ctx.corpus.docs as f64 >= MAX_CORPUS_DF
}

/// A template value, never a name: `COMPONENT_NAME`, `$STR_X`,
/// `MACRO_NAME_VALUE`.
///
/// The uppercase-with-underscore test names a macro metavariable, not
/// every uppercase token: real PBO prefixes such as `JAS_GPNVG18` are
/// names and must survive. A leading Arma config macro (`CSTRING(...)`)
/// is a template too.
pub(crate) fn is_placeholder(text: &str) -> bool {
    const MACRO_SEGMENTS: &[&str] = &[
        "NAME",
        "VALUE",
        "COMPONENT",
        "MACRO",
        "PLACEHOLDER",
        "TODO",
        "FIXME",
        "DEFAULT",
    ];
    const CONFIG_MACROS: &[&str] = &[
        "CSTRING", "QSTRING", "STRING", "NUMBER", "SCALAR", "ARRAY", "PATH", "ICON", "PICTURE",
        "MODEL",
    ];
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.starts_with("$STR_") {
        return true;
    }
    let first = trimmed
        .split(|c: char| !c.is_ascii_alphanumeric())
        .next()
        .unwrap_or("");
    if CONFIG_MACROS.contains(&first) {
        return true;
    }
    let macro_shape = trimmed.chars().count() >= 6
        && trimmed.contains('_')
        && !trimmed.chars().any(char::is_lowercase)
        && trimmed
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    macro_shape
        && trimmed
            .split('_')
            .any(|segment| MACRO_SEGMENTS.contains(&segment))
}

/// The grammatical head of a phrase: the last non-numeric, non-generic
/// token, else the longest token.
pub(crate) fn head_noun(phrase: &str) -> Option<String> {
    let normalised = normalise(phrase);
    if let Some(token) = normalised
        .tokens
        .iter()
        .rev()
        .find(|token| !is_numeric(token) && !is_generic_word(token) && is_alpha(token))
    {
        return Some(token.clone());
    }
    normalised
        .tokens
        .iter()
        .filter(|token| !is_numeric(token))
        .max_by_key(|token| token.chars().count())
        .cloned()
        .or_else(|| normalised.tokens.last().cloned())
}

fn phrase_text(evidence: &IdentityEvidence, source: PhraseSource) -> Option<String> {
    evidence
        .phrases
        .iter()
        .find(|phrase| phrase.source == source)
        .map(|phrase| phrase.text.clone())
}

/// The display name with the most surviving tokens. Plain names beat
/// names that kept an item-detail parenthetical, and the first name wins
/// a tie, so the result is stable.
fn strongest_display(evidence: &IdentityEvidence) -> Option<&IdentityPhrase> {
    let displays = || {
        evidence
            .phrases
            .iter()
            .filter(|phrase| phrase.source == PhraseSource::DisplayName)
    };
    let plain: Vec<&IdentityPhrase> = displays().filter(|p| !p.text.contains('(')).collect();
    let pool: Vec<&IdentityPhrase> = if plain.is_empty() {
        displays().collect()
    } else {
        plain
    };

    let mut best: Option<&IdentityPhrase> = None;
    for phrase in pool {
        let score = display_score(&phrase.text);
        if best
            .map(|current| score > display_score(&current.text))
            .unwrap_or(true)
        {
            best = Some(phrase);
        }
    }
    best
}

fn display_score(text: &str) -> usize {
    normalise(text)
        .tokens
        .iter()
        .filter(|token| {
            !is_numeric(token)
                && !is_generic_word(token)
                && is_alpha(token)
                && token.chars().count() >= 3
        })
        .count()
}

/// The leading distinctive word of a display name. Workshop rank
/// favours this word, so it becomes the tag query's second word.
fn modifier_token(phrase: &str) -> Option<String> {
    let normalised = normalise(phrase);
    normalised
        .tokens
        .iter()
        .find(|token| {
            !is_numeric(token)
                && !is_generic_word(token)
                && is_alpha(token)
                && token.chars().count() >= 3
        })
        .cloned()
        .or_else(|| head_noun(phrase))
}

fn prefix_query(evidence: &IdentityEvidence) -> Option<(String, PhraseSource)> {
    let prefix_phrase = phrase_text(evidence, PhraseSource::PrefixFamily);
    let (raw, source) = match prefix_phrase.as_deref() {
        Some(prefix) if !prefix.trim().is_empty() => (prefix, PhraseSource::PrefixFamily),
        _ => match evidence.prefix.as_deref() {
            Some(prefix) if !prefix.trim().is_empty() => (prefix, PhraseSource::PrefixFamily),
            _ if !evidence.stem.trim().is_empty() => (evidence.stem.as_str(), PhraseSource::Stem),
            _ => return None,
        },
    };
    if is_placeholder(raw) {
        return None;
    }

    let normalised = normalise(raw);
    let head = head_noun(raw);
    let family = normalised
        .tokens
        .iter()
        .find(|token| {
            !is_numeric(token)
                && !is_generic_word(token)
                && token.chars().count() >= 2
                && Some(token.as_str()) != head.as_deref()
        })
        .cloned();

    let text = match (family, head) {
        (Some(family), Some(head)) => format!("{family} {head}"),
        (None, Some(head)) => head,
        _ => return None,
    };
    Some((text, source))
}

fn class_query(class: &str) -> Option<(String, String)> {
    // Keep generic words in the query text: "common" is the useful half of
    // "rksla3_common". Only the anchor must be distinctive.
    let parts: Vec<String> = normalise(class)
        .tokens
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    let root = parts.first()?;
    let second = parts.get(1)?;
    let text = format!("{root} {second}");
    let anchor = anchor_token(&text)?;
    Some((text, anchor))
}

fn push_query(
    plan: &mut QueryPlan,
    text: String,
    kind: QueryKind,
    source: PhraseSource,
    ctx: &PlannerContext,
) {
    let text = text.trim().to_string();
    if let Some(anchor) = anchor_token(&text) {
        push_anchored(plan, text, kind, source, anchor, ctx);
    }
}

fn push_anchored(
    plan: &mut QueryPlan,
    text: String,
    kind: QueryKind,
    source: PhraseSource,
    anchor: String,
    ctx: &PlannerContext,
) {
    if plan.queries.len() >= MAX_QUERIES {
        return;
    }
    if !accepted(&text, &anchor, ctx) {
        return;
    }
    let text_key = normalise(&text).text;
    if plan
        .queries
        .iter()
        .any(|query| normalise(&query.text).text == text_key)
    {
        return;
    }
    plan.queries.push(PlannedQuery {
        text,
        kind,
        source,
        anchor,
    });
}

fn accepted(text: &str, anchor: &str, ctx: &PlannerContext) -> bool {
    if is_placeholder(text) {
        return false;
    }
    if !is_usable_anchor_with(anchor, ctx) {
        return false;
    }
    true
}

/// The query's one anchor: the head noun when it is distinctive, else
/// the longest distinctive token.
fn anchor_token(text: &str) -> Option<String> {
    let tokens = normalise(text).tokens;
    if let Some(head) = head_noun(text) {
        if is_usable_anchor(&head) {
            return Some(head);
        }
    }
    tokens
        .iter()
        .filter(|token| is_usable_anchor(token))
        .max_by_key(|token| token.chars().count())
        .cloned()
}

fn is_usable_anchor(token: &str) -> bool {
    token.chars().count() >= 4
        && !is_numeric(token)
        && !is_generic_word(token)
        && is_brand_token(token)
}

/// A brand token may carry a version or model number: `rksla3`, `bl500`,
/// `gpnvg18`. It must still contain a letter and contain no punctuation.
fn is_brand_token(token: &str) -> bool {
    token
        .chars()
        .all(|character| character.is_ascii_alphanumeric())
        && token
            .chars()
            .any(|character| character.is_ascii_alphabetic())
}

/// The learned anchor test: length, shape and corpus rarity.
fn is_usable_anchor_with(token: &str, ctx: &PlannerContext) -> bool {
    token.chars().count() >= 4
        && !is_numeric(token)
        && is_brand_token(token)
        && !is_learned_generic(token, ctx)
}

fn is_numeric(token: &str) -> bool {
    !token.is_empty() && token.chars().all(|c| c.is_ascii_digit())
}

fn is_alpha(token: &str) -> bool {
    token.chars().any(char::is_alphabetic)
}

#[cfg(test)]
mod tests {
    use super::super::evidence::{EvidenceKind, EvidenceString};
    use super::super::identity::IdentityPhrase;
    use super::*;

    fn phrase(text: &str, source: PhraseSource) -> IdentityPhrase {
        IdentityPhrase {
            text: text.to_string(),
            source,
            workshop_id: None,
        }
    }

    fn evidence(phrases: Vec<IdentityPhrase>) -> IdentityEvidence {
        IdentityEvidence {
            phrases,
            ..IdentityEvidence::default()
        }
    }

    fn empty_context() -> (CorpusStats, HashSet<String>) {
        (CorpusStats::default(), HashSet::new())
    }

    fn plan_with<'a>(
        evidence: &IdentityEvidence,
        corpus: &'a CorpusStats,
        groups: &'a HashSet<String>,
        pool: Option<&'a EvidencePool>,
    ) -> QueryPlan {
        plan_queries(
            evidence,
            &PlannerContext {
                corpus,
                group_terms: groups,
                pool,
            },
        )
    }

    fn plan_for(evidence: &IdentityEvidence) -> QueryPlan {
        let (corpus, groups) = empty_context();
        plan_with(evidence, &corpus, &groups, None)
    }

    #[test]
    fn plan_rejects_family_token_only() {
        let ev = evidence(vec![phrase("JAS", PhraseSource::PrefixFamily)]);

        assert!(plan_for(&ev).queries.is_empty());
    }

    #[test]
    fn plan_rejects_numeric_prefix_fragment() {
        let ev = evidence(vec![phrase("121_SOAR_Gear", PhraseSource::PrefixFamily)]);

        let plan = plan_for(&ev);

        assert!(plan.queries.iter().all(|query| query.text != "121"));
        assert_eq!(head_noun("121_SOAR_Gear").as_deref(), Some("soar"));
    }

    #[test]
    fn plan_caps_at_three_queries() {
        let ev = evidence(vec![
            phrase("BettIR Core", PhraseSource::CfgModsName),
            phrase("BIR", PhraseSource::TagAuthor),
            phrase("Diving Mask", PhraseSource::DisplayName),
            phrase("BettIR_Devices", PhraseSource::CfgPatchesClass),
            phrase("BettIR_Core", PhraseSource::PrefixFamily),
        ]);

        let plan = plan_for(&ev);

        assert!(plan.queries.len() <= MAX_QUERIES);
        assert_eq!(plan.queries.len(), MAX_QUERIES);
    }

    #[test]
    fn plan_orders_most_specific_first() {
        let ev = evidence(vec![
            phrase("BettIR Core", PhraseSource::CfgModsName),
            phrase("ADE", PhraseSource::TagAuthor),
            phrase("Diving Mask", PhraseSource::DisplayName),
            phrase("BettIR_Devices", PhraseSource::CfgPatchesClass),
        ]);

        let plan = plan_for(&ev);

        assert_eq!(
            plan.queries
                .iter()
                .map(|query| query.kind)
                .collect::<Vec<_>>(),
            [
                QueryKind::IdentityName,
                QueryKind::TagPlusHead,
                QueryKind::ClassRootPlusToken,
            ]
        );
        assert_eq!(plan.queries[0].source, PhraseSource::CfgModsName);
        assert_eq!(plan.queries[1].source, PhraseSource::TagAuthor);
        assert_eq!(plan.queries[2].source, PhraseSource::CfgPatchesClass);
    }

    #[test]
    fn plan_rejects_placeholder_shapes() {
        let ev = evidence(vec![
            phrase("COMPONENT_NAME", PhraseSource::CfgModsName),
            phrase("$STR_X", PhraseSource::StrToken),
            phrase("MACRO_NAME_VALUE", PhraseSource::Stem),
        ]);

        let plan = plan_for(&ev);

        assert!(plan.queries.is_empty());
    }

    #[test]
    fn plan_rejects_high_corpus_df_anchor() {
        let ev = evidence(vec![phrase("SOAR", PhraseSource::PrefixFamily)]);
        let mut df = HashMap::new();
        df.insert("soar".to_string(), 20);
        let corpus = CorpusStats { df, docs: 50 };
        let groups = HashSet::new();

        let plan = plan_with(&ev, &corpus, &groups, None);

        assert!(plan.queries.is_empty());
    }

    #[test]
    fn plan_head_noun_skips_numeric_and_generic() {
        assert_eq!(head_noun("121_SOAR_Gear").as_deref(), Some("soar"));
        assert_eq!(head_noun("BettIR Core").as_deref(), Some("bettir"));
        assert_eq!(head_noun("Diving Mask").as_deref(), Some("mask"));
    }

    #[test]
    fn plan_short_circuits_on_workshop_url() {
        let mut url = phrase(
            "https://steamcommunity.com/sharedfiles/filedetails/?id=1340701737",
            PhraseSource::WorkshopUrl,
        );
        url.workshop_id = Some("1340701737".to_string());
        let ev = evidence(vec![url, phrase("BettIR Core", PhraseSource::CfgModsName)]);

        let plan = plan_for(&ev);

        assert_eq!(plan.queries.len(), 1);
        assert_eq!(plan.queries[0].source, PhraseSource::WorkshopUrl);
        assert_eq!(plan.queries[0].anchor, "1340701737");
    }

    #[test]
    fn plan_builds_tag_plus_distinctive_modifier() {
        let ev = evidence(vec![
            phrase("ADE", PhraseSource::TagAuthor),
            phrase("Diving Mask", PhraseSource::DisplayName),
            phrase("Diving Bag", PhraseSource::DisplayName),
            phrase("ade_core", PhraseSource::CfgPatchesClass),
        ]);

        let plan = plan_for(&ev);

        assert_eq!(plan.queries[0].text, "Diving Mask");
        assert_eq!(plan.queries[1].text, "ADE diving");
        assert_eq!(plan.queries[1].anchor, "diving");
    }

    #[test]
    fn plan_prefix_keeps_distinctive_token_not_family_token() {
        let ev = evidence(vec![phrase("JAS_GPNVG18", PhraseSource::PrefixFamily)]);

        let plan = plan_for(&ev);

        assert_eq!(plan.queries.len(), 1);
        assert!(plan.queries[0].text.contains("gpnvg18"));
        assert_ne!(plan.queries[0].text.to_lowercase(), "jas");
    }

    fn pool_for(pbo: &str, texts: &[(&str, EvidenceKind)]) -> EvidencePool {
        EvidencePool {
            strings: texts
                .iter()
                .map(|(text, kind)| EvidenceString {
                    pbo: pbo.to_string(),
                    text: (*text).to_string(),
                    key: normalise(text).text,
                    kind: *kind,
                    token_df: 1,
                    pbo_df: 1,
                })
                .collect(),
            corpus: CorpusStats {
                df: HashMap::new(),
                docs: 4,
            },
            truncated: false,
        }
    }

    #[test]
    fn planner_uses_dynamic_phrase_when_no_typed_seed() {
        let mut ev = evidence(vec![phrase("JAS", PhraseSource::PrefixFamily)]);
        ev.pbo = "mod.pbo".to_string();
        let (corpus, groups) = empty_context();
        let pool = pool_for(
            "mod.pbo",
            &[("Advanced Diving Environment", EvidenceKind::Quoted)],
        );

        let plan = plan_with(&ev, &corpus, &groups, Some(&pool));

        assert_eq!(plan.queries.len(), 1);
        assert_eq!(plan.queries[0].text, "Advanced Diving Environment");
        assert_eq!(plan.queries[0].source, PhraseSource::Dynamic);
        assert_eq!(plan.queries[0].anchor, "environment");
    }

    #[test]
    fn planner_never_uses_structural_stopword_as_anchor() {
        let mut ev = evidence(Vec::new());
        ev.pbo = "mod.pbo".to_string();
        let (corpus, groups) = empty_context();
        let pool = pool_for(
            "mod.pbo",
            &[
                ("author name value", EvidenceKind::Assignment),
                ("config displayname text", EvidenceKind::Quoted),
            ],
        );

        let plan = plan_with(&ev, &corpus, &groups, Some(&pool));

        assert!(plan.queries.is_empty());
        assert!(plan
            .queries
            .iter()
            .all(|query| !is_structural_stop(&query.anchor)));
        assert!(is_structural_stop("config"));
        assert!(is_structural_stop("displayname"));
        assert!(!is_structural_stop("diving"));
    }

    fn learned_evidence(pbo: &str, phrases: Vec<IdentityPhrase>) -> IdentityEvidence {
        IdentityEvidence {
            pbo: pbo.to_string(),
            phrases,
            ..IdentityEvidence::default()
        }
    }

    #[test]
    fn planner_prefers_dynamic_phrase_over_display_name() {
        let ev = learned_evidence(
            "bl500.pbo",
            vec![phrase("Two-Man CamoNet", PhraseSource::DisplayName)],
        );
        let (corpus, groups) = empty_context();
        let pool = pool_for("bl500.pbo", &[("Skywatch BL500", EvidenceKind::Assignment)]);

        let plan = plan_with(&ev, &corpus, &groups, Some(&pool));

        assert_eq!(plan.queries[0].text, "Skywatch BL500");
        assert_eq!(plan.queries[0].source, PhraseSource::Dynamic);
        assert_ne!(plan.queries[0].text, "Two-Man CamoNet");
    }

    #[test]
    fn planner_keeps_display_name_as_later_query() {
        let ev = learned_evidence(
            "bl500.pbo",
            vec![phrase("Two-Man CamoNet", PhraseSource::DisplayName)],
        );
        let (corpus, groups) = empty_context();
        let pool = pool_for("bl500.pbo", &[("Skywatch BL500", EvidenceKind::Assignment)]);

        let plan = plan_with(&ev, &corpus, &groups, Some(&pool));

        assert_eq!(plan.queries[0].source, PhraseSource::Dynamic);
        assert_eq!(plan.queries[1].text, "Two-Man CamoNet");
        assert_eq!(plan.queries[1].source, PhraseSource::DisplayName);
    }

    #[test]
    fn planner_without_pool_still_uses_display_name() {
        let ev = learned_evidence(
            "ade.pbo",
            vec![phrase("Diving Mask", PhraseSource::DisplayName)],
        );
        let (corpus, groups) = empty_context();

        let plan = plan_with(&ev, &corpus, &groups, None);

        assert_eq!(plan.queries[0].text, "Diving Mask");
        assert_eq!(plan.queries[0].source, PhraseSource::DisplayName);
    }

    #[test]
    fn planner_dedupes_dynamic_and_display_text() {
        let ev = learned_evidence(
            "dedupe.pbo",
            vec![phrase("Sniper-Tools", PhraseSource::DisplayName)],
        );
        let (corpus, groups) = empty_context();
        let pool = pool_for("dedupe.pbo", &[("Sniper Tools", EvidenceKind::Assignment)]);

        let plan = plan_with(&ev, &corpus, &groups, Some(&pool));

        assert_eq!(plan.queries.len(), 1);
        assert_eq!(normalise(&plan.queries[0].text).text, "sniper tools");
        assert_eq!(plan.queries[0].source, PhraseSource::Dynamic);
    }

    #[test]
    fn planner_caps_at_three_queries() {
        let ev = learned_evidence(
            "bettir.pbo",
            vec![
                phrase("ADE", PhraseSource::TagAuthor),
                phrase("Diving Mask", PhraseSource::DisplayName),
                phrase("BettIR_Devices", PhraseSource::CfgPatchesClass),
            ],
        );
        let (corpus, groups) = empty_context();
        let pool = pool_for(
            "bettir.pbo",
            &[("Skywatch BL500", EvidenceKind::Assignment)],
        );

        let plan = plan_with(&ev, &corpus, &groups, Some(&pool));

        assert_eq!(plan.queries.len(), MAX_QUERIES);
        assert_eq!(
            plan.queries
                .iter()
                .map(|query| query.source)
                .collect::<Vec<_>>(),
            [
                PhraseSource::Dynamic,
                PhraseSource::DisplayName,
                PhraseSource::TagAuthor,
            ]
        );
    }
}

#[cfg(test)]
mod cba_plan_tests {
    use super::*;
    use crate::investigate::identity::{IdentityEvidence, IdentityPhrase, PhraseSource};
    use std::collections::HashSet;

    #[test]
    fn planner_uses_cba_category_as_q1() {
        let evidence = IdentityEvidence {
            pbo: "ade_core.pbo".to_string(),
            phrases: vec![
                IdentityPhrase {
                    text: "Advanced Diving Environment".to_string(),
                    source: PhraseSource::CbaOptionName,
                    workshop_id: None,
                },
                IdentityPhrase {
                    text: "Diving Mask".to_string(),
                    source: PhraseSource::DisplayName,
                    workshop_id: None,
                },
            ],
            ..Default::default()
        };
        let corpus = CorpusStats::default();
        let terms = HashSet::new();
        let ctx = PlannerContext {
            corpus: &corpus,
            group_terms: &terms,
            pool: None,
        };

        let plan = plan_queries(&evidence, &ctx);

        assert_eq!(plan.queries[0].text, "Advanced Diving Environment");
        assert_eq!(plan.queries[0].source, PhraseSource::CbaOptionName);
        assert!(plan.queries.iter().any(|query| query.text == "Diving Mask"));
    }
}

#[cfg(test)]
mod corpus_tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uksfta-plan-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn corpus_from_multiple_dirs_counts_documents_once() {
        let first = temp_dir("corpus-first");
        let second = temp_dir("corpus-second");
        for name in ["ade_core.pbo", "core_core.pbo"] {
            std::fs::write(first.join(name), b"").unwrap();
        }
        std::fs::write(second.join("ade_shared.pbo"), b"").unwrap();

        let corpus = build_corpus_stats_many(&[first, second]);

        assert_eq!(corpus.docs, 3);
        assert_eq!(corpus.df["ade"], 2);
        // A word repeated in one name counts once for that document.
        assert_eq!(corpus.df["core"], 2);
    }

    #[test]
    fn corpus_single_dir_wrapper_matches_one_directory() {
        let dir = temp_dir("corpus-single");
        std::fs::write(dir.join("ade_core.pbo"), b"").unwrap();

        let corpus = build_corpus_stats(&dir);

        assert_eq!(corpus, build_corpus_stats_many(&[dir]));
    }
}

#[cfg(test)]
mod class_query_tests {
    use super::*;

    #[test]
    fn class_query_splits_hyphen_and_underscore() {
        let (text, anchor) = class_query("rksla3-common").unwrap();

        assert_eq!(text, "rksla3 common");
        assert_eq!(anchor, "rksla3");
    }
}

#[cfg(test)]
mod brand_anchor_tests {
    use super::*;

    #[test]
    fn anchor_accepts_alphanumeric_brand() {
        assert_eq!(anchor_token("bl500"), Some("bl500".to_string()));
        assert_eq!(anchor_token("gpnvg18"), Some("gpnvg18".to_string()));
        assert_eq!(anchor_token("rksla3 common"), Some("rksla3".to_string()));
    }

    #[test]
    fn anchor_rejects_pure_numeric() {
        assert_eq!(anchor_token("123456"), None);
    }
}

#[cfg(test)]
mod dedupe_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn distinct_text_keeps_same_anchor_query() {
        let corpus = CorpusStats::default();
        let terms = HashSet::new();
        let ctx = PlannerContext {
            corpus: &corpus,
            group_terms: &terms,
            pool: None,
        };
        let mut plan = QueryPlan::default();
        push_anchored(
            &mut plan,
            "rksla3".to_string(),
            QueryKind::TagPlusHead,
            PhraseSource::PrefixFamily,
            "rksla3".to_string(),
            &ctx,
        );
        push_anchored(
            &mut plan,
            "rksla3 common".to_string(),
            QueryKind::ClassRootPlusToken,
            PhraseSource::CfgPatchesClass,
            "rksla3".to_string(),
            &ctx,
        );

        assert_eq!(plan.queries.len(), 2);
    }
}
