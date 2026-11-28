//! Candidate scoring and the confirmation gate.
//!
//! Two independent scores decide whether a Workshop result is the PBO's
//! origin. `M` measures how well a title matches the query, with the
//! anchor token as a hard gate. `Q` measures whether the query was
//! specific enough to trust. `C = M * Q` orders candidates, but
//! confirmation also needs corroboration from the PBO's own evidence.
//! Content corroboration can rescue a missing head noun or a thin margin.
//! It never rescues a title that fails [`MIN_M`].
//!
//! Nothing here ranks by popularity. Subscriptions and views only break
//! ties between candidates that already match the text.

use std::collections::BTreeSet;

use super::browse::BrowseResult;
use super::evidence::{EvidenceKind, EvidencePool};
use super::identity::IdentityEvidence;
use super::normalise::{normalise, Normalised};
use super::plan::{head_noun, is_structural_stop, CorpusStats, PlannedQuery};

/// A title must reach this match score before it can be confirmed.
pub(crate) const MIN_M: f64 = 0.80;
/// A query must be this specific before its answer can be confirmed.
pub(crate) const MIN_Q: f64 = 0.62;
/// The runner-up must trail by at least this much.
pub(crate) const MIN_MARGIN: f64 = 0.15;
/// A title that misses the anchor token cannot exceed this score.
pub(crate) const ANCHOR_CAP: f64 = 0.40;
/// Shared rare tokens needed to confirm on content alone.
pub(crate) const EVIDENCE_MIN_TOKENS: usize = 3;
/// At least one shared token must be this long.
pub(crate) const EVIDENCE_MIN_LEN: usize = 6;
/// Rare-token document frequency ceiling.
pub(crate) const RARE_TOKEN_DF: usize = 2;
/// A two-word run of initials carries no identity.
const ACRONYM_MIN_LEN: usize = 3;
const ACRONYM_MAX_LEN: usize = 6;
/// A shared token counts from any context only when it is this long.
const EVIDENCE_BARE_LEN: usize = 6;

/// How a title matched a query.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct MatchScore {
    pub m: f64,
    pub cover: f64,
    pub order: f64,
    pub head: f64,
    pub anchor_matched: bool,
}

/// How specific a query was, independent of any candidate.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct QueryScore {
    pub q: f64,
    pub q_len: f64,
    pub q_shape: f64,
    pub q_df: f64,
    pub q_count: f64,
    pub total_count: u64,
    pub count_known: bool,
}

/// Signals from the PBO that agree with a candidate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Corroboration {
    pub url_id_match: bool,
    pub author_match: bool,
    pub dep_match: bool,
    pub stem_in_description: bool,
    /// The item page's description carries a phrase from this PBO. Set by
    /// a caller that fetched the page, not by [`corroborate`].
    pub description_match: bool,
    pub evidence_overlap: usize,
    pub evidence_tokens: Vec<String>,
}

impl Corroboration {
    /// Content alone is enough when three rare tokens are shared and at
    /// least one is distinctive enough to be a name.
    pub(crate) fn evidence_gate(&self) -> bool {
        self.evidence_overlap >= EVIDENCE_MIN_TOKENS
            && self
                .evidence_tokens
                .iter()
                .any(|token| token.chars().count() >= EVIDENCE_MIN_LEN)
    }

    pub(crate) fn any(&self) -> bool {
        self.url_id_match
            || self.author_match
            || self.dep_match
            || self.stem_in_description
            || self.description_match
            || self.evidence_gate()
    }
}

/// One scored candidate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RankedCandidate {
    pub id: String,
    pub title: String,
    pub file_type: u32,
    pub m: f64,
    pub c: f64,
    pub head: f64,
    /// Whether the query's anchor token appears in the title. A capped
    /// score alone does not prove it, so the decision reads this.
    pub anchor_matched: bool,
    pub corroboration: Corroboration,
}

/// The outcome for one PBO.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum ResolutionStatus {
    Confirmed,
    Ambiguous,
    NoDistinctiveQuery,
    NoResult,
    RateLimited,
    Unresolved,
}

impl ResolutionStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ResolutionStatus::Confirmed => "confirmed",
            ResolutionStatus::Ambiguous => "ambiguous",
            ResolutionStatus::NoDistinctiveQuery => "no-query",
            ResolutionStatus::NoResult => "no-result",
            ResolutionStatus::RateLimited => "rate-limited",
            ResolutionStatus::Unresolved => "unresolved",
        }
    }
}

/// The decision for one PBO. A title exists only when confirmed.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Resolution {
    pub status: ResolutionStatus,
    pub id: Option<String>,
    pub title: Option<String>,
    pub m: f64,
    pub q: f64,
    pub c: f64,
    pub margin: f64,
    pub query: Option<String>,
    pub matched_by: Vec<String>,
}

impl Resolution {
    fn status_only(status: ResolutionStatus, query: Option<String>) -> Self {
        Self {
            status,
            id: None,
            title: None,
            m: 0.0,
            q: 0.0,
            c: 0.0,
            margin: 0.0,
            query,
            matched_by: Vec::new(),
        }
    }
}

/// Score a title against a query phrase.
pub(crate) fn score_match(item: &BrowseResult, phrase: &Normalised, anchor: &str) -> MatchScore {
    let title = normalise(&item.title);
    if contains_squashed(&title.tokens, &phrase.squashed) {
        return MatchScore {
            m: 1.0,
            cover: 1.0,
            order: 1.0,
            head: 1.0,
            anchor_matched: true,
        };
    }

    let positions: Vec<usize> = phrase
        .tokens
        .iter()
        .filter_map(|token| token_position(&title.tokens, token))
        .collect();

    let cover = if phrase.tokens.is_empty() {
        0.0
    } else {
        positions.len() as f64 / phrase.tokens.len() as f64
    };
    let order = if positions.len() < phrase.tokens.len() {
        0.0
    } else if positions.windows(2).all(|pair| pair[0] < pair[1]) {
        1.0
    } else {
        0.5
    };
    let head = if title_has_head(&title.tokens, &phrase.text) {
        1.0
    } else {
        0.0
    };
    let anchor_matched = anchor_token_matches(&title.tokens, anchor);

    let mut m = 0.55 * cover + 0.25 * order + 0.20 * head;
    if !anchor_matched {
        m = m.min(ANCHOR_CAP);
    }

    MatchScore {
        m: m.min(1.0),
        cover,
        order,
        head,
        anchor_matched,
    }
}

/// Whether the phrase's grammatical head noun appears in the title.
fn title_has_head(title_tokens: &[String], phrase: &str) -> bool {
    match head_noun(phrase) {
        Some(head) => token_position(title_tokens, &head).is_some(),
        None => false,
    }
}

/// Whether the query's anchor token appears in the title. An empty anchor
/// places no constraint.
fn anchor_token_matches(title_tokens: &[String], anchor: &str) -> bool {
    match normalise(anchor).tokens.into_iter().next() {
        Some(token) => token_position(title_tokens, &token).is_some(),
        None => true,
    }
}

/// Where a query token matches the title: the token itself, else the first
/// token of a contiguous run whose initials spell it.
fn token_position(title_tokens: &[String], token: &str) -> Option<usize> {
    title_tokens
        .iter()
        .position(|candidate| candidate == token)
        .or_else(|| acronym_position(title_tokens, token))
        .or_else(|| brand_position(title_tokens, token))
}

/// Where a brand token differs from a title word only by a short suffix or
/// prefix. Arma 3 brands often add a version tag: `rksla3` for `RKSL`,
/// `arma3` for `arma`. Four characters minimum and at most two extra
/// characters, so unrelated words do not collapse into each other.
fn brand_position(title_tokens: &[String], token: &str) -> Option<usize> {
    let length = token.chars().count();
    if length < 4 {
        return None;
    }
    title_tokens.iter().position(|candidate| {
        let other = candidate.chars().count();
        if other < 4 {
            return false;
        }
        let (short, long) = if length <= other {
            (token, candidate.as_str())
        } else {
            (candidate.as_str(), token)
        };
        long.starts_with(short) && long.chars().count() - short.chars().count() <= 2
    })
}

/// Where a run of adjacent title tokens spells `token` from its initials.
///
/// One letter per word and no gaps, so a real acronym binds to the title it
/// abbreviates and scattered initials never match.
fn acronym_position(title_tokens: &[String], token: &str) -> Option<usize> {
    let letters = token.chars().count();
    if !(ACRONYM_MIN_LEN..=ACRONYM_MAX_LEN).contains(&letters)
        || !token.chars().all(|c| c.is_ascii_alphabetic())
    {
        return None;
    }
    (0..title_tokens.len().saturating_sub(letters - 1))
        .find(|&start| initials(&title_tokens[start..start + letters]) == token)
}

fn initials(tokens: &[String]) -> String {
    tokens
        .iter()
        .filter_map(|token| token.chars().next())
        .collect()
}

/// Score the query itself: length, shape, learned rarity and breadth.
pub(crate) fn score_query(
    query: &str,
    total_count: Option<u64>,
    corpus: &CorpusStats,
) -> QueryScore {
    let normalised = normalise(query);
    let q_len = normalised.squashed.chars().count().min(24) as f64 / 24.0;
    let q_shape = if normalised.tokens.len() >= 2
        && normalised
            .tokens
            .iter()
            .any(|token| token.chars().count() >= 5)
    {
        1.0
    } else if normalised.tokens.len() == 1 && normalised.tokens[0].chars().count() >= 6 {
        0.6
    } else if normalised.tokens.len() == 1 && normalised.tokens[0].chars().count() >= 3 {
        0.3
    } else {
        0.0
    };

    let df = rarest_df(&normalised.tokens, corpus);
    let ratio = if corpus.docs == 0 {
        0.0
    } else {
        df as f64 / corpus.docs as f64
    };
    let q_df = 1.0 - (ratio / 0.10).min(1.0);

    let (q_count, count_known) = match total_count {
        Some(count) => {
            // Steam matches query tokens with OR, so a two-word query
            // returns every item that carries either word. Breadth alone
            // does not make an exact multi-token title weak: a query whose
            // tokens are all rare keeps half the breadth weight.
            let breadth = 1.0 - (count as f64 / 5000.0).min(1.0);
            let floor_applies = normalised.tokens.len() >= 2 && q_df >= 0.75;
            let q_count = if floor_applies {
                breadth.max(0.5)
            } else {
                breadth
            };
            (q_count, true)
        }
        None => (0.5, false),
    };

    QueryScore {
        q: 0.25 * q_len + 0.15 * q_shape + 0.25 * q_df + 0.35 * q_count,
        q_len,
        q_shape,
        q_df,
        q_count,
        total_count: total_count.unwrap_or(0),
        count_known,
    }
}

/// Compare a candidate against everything known about the PBO.
pub(crate) fn corroborate(
    evidence: &IdentityEvidence,
    item: &BrowseResult,
    pool: Option<&EvidencePool>,
) -> Corroboration {
    let title = normalise(&item.title).squashed;
    let description = normalise(&item.short_description).squashed;
    let creator = normalise(&item.creator).squashed;

    let url_id_match = evidence
        .phrases
        .iter()
        .any(|phrase| phrase.workshop_id.as_deref() == Some(item.id.as_str()));

    let author_match = [
        evidence.cfgmods_author.as_deref(),
        evidence.cfgpatches_author.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|author| !author.starts_with("$STR_"))
    .map(|author| normalise(author).squashed)
    .any(|author| {
        author.chars().count() >= 3
            && creator.chars().count() >= 3
            && (author.contains(&creator) || creator.contains(&author))
    });

    let dep_match = evidence.required_addons.iter().any(|dependency| {
        let root = normalise(dependency)
            .tokens
            .first()
            .cloned()
            .unwrap_or_default();
        root.chars().count() >= 3 && (title.contains(&root) || description.contains(&root))
    });

    let stem = normalise(&evidence.stem).squashed;
    let stem_in_description = stem.chars().count() >= 6 && description.contains(&stem);

    let (evidence_overlap, evidence_tokens) = match pool {
        Some(pool) => shared_rare_tokens(evidence, item, pool),
        None => (0, Vec::new()),
    };

    Corroboration {
        url_id_match,
        author_match,
        dep_match,
        stem_in_description,
        description_match: false,
        evidence_overlap,
        evidence_tokens,
    }
}

/// Score and order every candidate. Popularity only breaks a tie.
pub(crate) fn rank(
    items: &[BrowseResult],
    phrase: &Normalised,
    anchor: &str,
    evidence: &IdentityEvidence,
    pool: Option<&EvidencePool>,
) -> Vec<RankedCandidate> {
    let mut scored: Vec<(f64, RankedCandidate)> = items
        .iter()
        .map(|item| {
            let score = score_match(item, phrase, anchor);
            let corroboration = corroborate(evidence, item, pool);
            (
                score.m,
                RankedCandidate {
                    id: item.id.clone(),
                    title: item.title.clone(),
                    file_type: item.file_type,
                    m: score.m,
                    c: score.m,
                    head: score.head,
                    anchor_matched: score.anchor_matched,
                    corroboration,
                },
            )
        })
        .collect();

    // Popularity never orders candidates. Equal scores stay equal and
    // are ordered by id only so the output is deterministic.
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.id.cmp(&b.1.id))
    });
    scored.into_iter().map(|(_, candidate)| candidate).collect()
}

/// Decide one PBO from its ranked candidates.
pub(crate) fn decide(
    ranked: &[RankedCandidate],
    query: &PlannedQuery,
    q: &QueryScore,
    url_id: Option<&str>,
) -> Resolution {
    if let Some(url_id) = url_id {
        if let Some(candidate) = ranked.iter().find(|candidate| candidate.id == url_id) {
            let second = ranked
                .iter()
                .filter(|other| other.id != candidate.id)
                .map(|other| other.m)
                .fold(0.0_f64, f64::max);
            return Resolution {
                status: ResolutionStatus::Confirmed,
                id: Some(candidate.id.clone()),
                title: Some(candidate.title.clone()),
                m: candidate.m,
                q: q.q,
                c: 1.0,
                margin: (candidate.m - second).max(0.0),
                query: Some(query.text.clone()),
                matched_by: vec!["workshop_url".to_string()],
            };
        }
    }

    let Some(best) = ranked.first() else {
        return Resolution::status_only(ResolutionStatus::NoResult, Some(query.text.clone()));
    };
    let second = ranked.get(1).map(|candidate| candidate.m).unwrap_or(0.0);
    let margin = (best.m - second).max(0.0);

    let title_gates = best.file_type == 0
        && best.m >= MIN_M
        && best.head >= 1.0
        && q.q >= MIN_Q
        && margin >= MIN_MARGIN
        && best.corroboration.any();
    // Content corroboration may rescue a missing head noun or a thin
    // margin, but never a weak title match: a title below `MIN_M` did not
    // name the item, however many common words it shares. It may not
    // bypass query specificity either, and a tie is never confirmed.
    let evidence_gate = best.file_type == 0
        && best.m >= MIN_M
        && best.anchor_matched
        && q.q >= MIN_Q
        && margin >= MIN_MARGIN
        && best.corroboration.evidence_gate();

    if title_gates || evidence_gate {
        let mut matched_by = Vec::new();
        if best.corroboration.url_id_match {
            matched_by.push("workshop_url".to_string());
        }
        if best.corroboration.author_match {
            matched_by.push("author".to_string());
        }
        if best.corroboration.dep_match {
            matched_by.push("dependency".to_string());
        }
        if best.corroboration.stem_in_description {
            matched_by.push("stem".to_string());
        }
        if best.corroboration.description_match {
            matched_by.push("item_description".to_string());
        }
        if best.corroboration.evidence_gate() {
            matched_by.push("evidence".to_string());
            matched_by.extend(best.corroboration.evidence_tokens.iter().cloned());
        }
        if matched_by.is_empty() {
            matched_by.push("title".to_string());
        }
        return Resolution {
            status: ResolutionStatus::Confirmed,
            id: Some(best.id.clone()),
            title: Some(best.title.clone()),
            m: best.m,
            q: q.q,
            c: best.m * q.q,
            margin,
            query: Some(query.text.clone()),
            matched_by,
        };
    }

    let mut resolution =
        Resolution::status_only(ResolutionStatus::Ambiguous, Some(query.text.clone()));
    if let Some(title) = shared_title(ranked, best.m) {
        resolution.matched_by = vec![format!("shared_title:{title}")];
    }
    resolution.m = best.m;
    resolution.q = q.q;
    resolution.c = best.m * q.q;
    resolution.margin = margin;
    resolution
}

/// The title words shared by every candidate tied with the best score.
/// Five ADE variants all score 1.00; their common name is still useful
/// even though the id is not. Two shared words are the minimum, and one
/// must be distinctive.
fn shared_title(ranked: &[RankedCandidate], best: f64) -> Option<String> {
    let tied: Vec<&RankedCandidate> = ranked
        .iter()
        .filter(|candidate| candidate.file_type == 0 && (candidate.m - best).abs() < f64::EPSILON)
        .take(5)
        .collect();
    if tied.len() < 2 {
        return None;
    }

    let mut common: Option<std::collections::BTreeSet<String>> = None;
    for candidate in &tied {
        let tokens: std::collections::BTreeSet<String> = normalise(&candidate.title)
            .tokens
            .into_iter()
            .filter(|token| token.len() >= 3 && !is_structural_stop(token))
            .collect();
        common = Some(match common {
            None => tokens,
            Some(existing) => existing.intersection(&tokens).cloned().collect(),
        });
    }
    let common = common?;
    if common.len() < 2 || !common.iter().any(|token| token.chars().count() >= 6) {
        return None;
    }

    let words: Vec<String> = tied[0]
        .title
        .split_whitespace()
        .filter(|word| common.contains(&normalise(word).squashed))
        .map(str::to_string)
        .collect();
    if words.len() < 2 {
        return None;
    }
    Some(words.join(" "))
}

/// Every rare token the PBO's evidence shares with the candidate text.
///
/// A token is rare when the learned corpus sees it in at most
/// [`RARE_TOKEN_DF`] PBOs. Structural stopwords never count: they carry no
/// identity, so an overlap built only from them cannot corroborate.
///
/// Equal weight per shared token lets a generic lowercase fragment
/// corroborate on its own, so a shared token counts only when its source
/// was a quoted or assigned value written with a capital letter, or when
/// it is long enough to be a name whatever its context.
fn shared_rare_tokens(
    evidence: &IdentityEvidence,
    item: &BrowseResult,
    pool: &EvidencePool,
) -> (usize, Vec<String>) {
    let candidate: BTreeSet<String> =
        normalise(&format!("{} {}", item.title, item.short_description))
            .tokens
            .into_iter()
            .collect();
    let mut shared = BTreeSet::new();

    for string in pool
        .strings
        .iter()
        .filter(|string| string.pbo == evidence.pbo)
    {
        let capitalised = matches!(string.kind, EvidenceKind::Quoted | EvidenceKind::Assignment)
            && string.text.chars().any(char::is_uppercase);
        for token in normalise(&string.text).tokens {
            let df = pool.corpus.df.get(&token).copied().unwrap_or(0);
            if df > RARE_TOKEN_DF || is_structural_stop(&token) || !candidate.contains(&token) {
                continue;
            }
            if capitalised || token.chars().count() >= EVIDENCE_BARE_LEN {
                shared.insert(token);
            }
        }
    }

    let tokens: Vec<String> = shared.into_iter().collect();
    (tokens.len(), tokens)
}

fn rarest_df(tokens: &[String], corpus: &CorpusStats) -> usize {
    tokens
        .iter()
        .map(|token| corpus.df.get(token).copied().unwrap_or(0))
        .min()
        .unwrap_or(0)
}

/// Whether a contiguous run of title tokens concatenates to the phrase.
///
/// A boundary-aligned squash match is the strongest possible match: the
/// whole phrase sits in the title, whatever the punctuation between the
/// tokens (`GPNVG-18` against `GPNVG18`).
fn contains_squashed(tokens: &[String], squashed: &str) -> bool {
    if squashed.is_empty() {
        return false;
    }
    for start in 0..tokens.len() {
        let mut accumulated = String::new();
        for token in &tokens[start..] {
            accumulated.push_str(token);
            if accumulated == squashed {
                return true;
            }
            if accumulated.chars().count() >= squashed.chars().count() {
                break;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigate::browse::BrowseResult;
    use crate::investigate::evidence::{EvidenceKind, EvidenceString};
    use crate::investigate::identity::PhraseSource;
    use crate::investigate::plan::QueryKind;

    fn item(id: &str, title: &str, file_type: u32) -> BrowseResult {
        BrowseResult {
            id: id.to_string(),
            title: title.to_string(),
            short_description: String::new(),
            tags: Vec::new(),
            subscriptions: 10,
            lifetime_subscriptions: 10,
            views: 10,
            favorited: 1,
            time_created: 0,
            time_updated: 0,
            file_size: String::new(),
            num_children: 0,
            children: Vec::new(),
            star_rating: 0.0,
            total_votes: 0,
            creator: "ADE Team".to_string(),
            file_type,
        }
    }

    fn evidence(author: Option<&str>) -> IdentityEvidence {
        IdentityEvidence {
            pbo: "ade_core.pbo".to_string(),
            cfgpatches_author: author.map(str::to_string),
            stem: "ade_core".to_string(),
            ..Default::default()
        }
    }

    fn query(text: &str, anchor: &str) -> PlannedQuery {
        PlannedQuery {
            text: text.to_string(),
            kind: QueryKind::IdentityName,
            source: PhraseSource::CfgModsName,
            anchor: anchor.to_string(),
        }
    }

    fn score(text: &str) -> QueryScore {
        score_query(text, Some(200), &CorpusStats::default())
    }

    fn pool_of(pbo: &str, entries: &[(&str, EvidenceKind)]) -> EvidencePool {
        EvidencePool {
            strings: entries
                .iter()
                .map(|(text, kind)| EvidenceString {
                    pbo: pbo.to_string(),
                    text: text.to_string(),
                    key: normalise(text).text,
                    kind: *kind,
                    token_df: 1,
                    pbo_df: 1,
                })
                .collect(),
            corpus: CorpusStats::default(),
            truncated: false,
        }
    }

    #[test]
    fn acronym_credits_contiguous_initials() {
        let score = score_match(
            &item("1", "Advanced Diving Environment", 0),
            &normalise("ade environment"),
            "environment",
        );

        assert_eq!(score.cover, 1.0);
        assert_eq!(score.m, 1.0);
    }

    #[test]
    fn acronym_requires_three_letters() {
        let score = score_match(
            &item("1", "Advanced Diving Equipment", 0),
            &normalise("ad equipment"),
            "equipment",
        );

        assert_eq!(score.cover, 0.5);
    }

    #[test]
    fn acronym_rejects_scattered_initials() {
        let score = score_match(
            &item("1", "Advanced Bronze Diving Environment", 0),
            &normalise("ade"),
            "ade",
        );

        assert_eq!(score.cover, 0.0);
        assert!(!score.anchor_matched);
    }

    #[test]
    fn acronym_requires_three_word_run() {
        let score = score_match(&item("1", "Mister H's", 0), &normalise("mrh"), "mrh");

        assert_eq!(score.cover, 0.0);
        assert!(!score.anchor_matched);
    }

    #[test]
    fn acronym_matches_anchor() {
        let score = score_match(
            &item("1", "French Forces Army Addons", 0),
            &normalise("FFAA"),
            "FFAA",
        );

        assert!(score.anchor_matched);
        assert!(score.m > ANCHOR_CAP);
    }

    #[test]
    fn evidence_counts_quoted_capitalised_token() {
        let evidence = evidence(None);
        let pool = pool_of(&evidence.pbo, &[("Dive Mask", EvidenceKind::Quoted)]);

        let (overlap, tokens) =
            shared_rare_tokens(&evidence, &item("1", "Diving Mask Set", 0), &pool);

        assert_eq!(overlap, 1);
        assert_eq!(tokens, ["mask"]);
    }

    #[test]
    fn evidence_ignores_bare_short_token() {
        let evidence = evidence(None);
        let pool = pool_of(
            &evidence.pbo,
            &[
                ("dive mask", EvidenceKind::Bare),
                ("dive mask", EvidenceKind::Quoted),
            ],
        );

        let (overlap, tokens) =
            shared_rare_tokens(&evidence, &item("1", "Diving Mask Set", 0), &pool);

        assert_eq!(overlap, 0);
        assert!(tokens.is_empty());
    }

    #[test]
    fn evidence_still_counts_six_char_bare_token() {
        let evidence = evidence(None);
        let pool = pool_of(&evidence.pbo, &[("diving mask", EvidenceKind::Bare)]);

        let (overlap, tokens) =
            shared_rare_tokens(&evidence, &item("1", "Diving Mask Set", 0), &pool);

        assert_eq!(overlap, 1);
        assert_eq!(tokens, ["diving"]);
    }

    #[test]
    fn score_full_squash_is_one() {
        let score = score_match(
            &item("1", "Jason's GPNVG-18 Night Vision", 0),
            &normalise("GPNVG18"),
            "GPNVG18",
        );

        assert_eq!(score.m, 1.0);
        assert!(score.anchor_matched);
    }

    #[test]
    fn score_caps_at_point_four_without_anchor() {
        let score = score_match(
            &item("1", "Diving Equipment Pack", 0),
            &normalise("Diving Mask"),
            "mask",
        );

        assert_eq!(score.head, 0.0);
        assert!(score.m <= ANCHOR_CAP);
    }

    #[test]
    fn score_order_penalises_reversed_tokens() {
        let score = score_match(
            &item("1", "Bravo Alpha", 0),
            &normalise("Alpha Bravo"),
            "alpha",
        );

        assert_eq!(score.order, 0.5);
        assert!(score.m < 1.0);
    }

    #[test]
    fn score_head_mismatch_blocks_confirmation() {
        let ranked = rank(
            &[item("1", "Advanced Diving Suit", 0)],
            &normalise("Advanced Diving Environment"),
            "environment",
            &evidence(Some("ADE")),
            None,
        );

        assert_eq!(ranked[0].head, 0.0);
        assert!(ranked[0].m < MIN_M);

        let decision = decide(
            &ranked,
            &query("Advanced Diving Environment", "environment"),
            &score("Advanced Diving Environment"),
            None,
        );

        assert_ne!(decision.status, ResolutionStatus::Confirmed);
        assert!(decision.title.is_none());
    }

    #[test]
    fn query_score_rejects_family_token() {
        let corpus = CorpusStats {
            docs: 100,
            df: [("jas".to_string(), 20)].into_iter().collect(),
        };

        assert!(score_query("JAS", Some(40), &corpus).q < MIN_Q);
    }

    #[test]
    fn query_score_accepts_distinctive_phrase() {
        assert!(score_query("GPNVG18", Some(40), &CorpusStats::default()).q >= MIN_Q);
    }

    #[test]
    fn query_score_penalises_broad_result_set() {
        assert!(score_query("Mod", Some(9000), &CorpusStats::default()).q < MIN_Q);
    }

    #[test]
    fn query_score_handles_missing_total_count() {
        let score = score_query("GPNVG18", None, &CorpusStats::default());

        assert!(!score.count_known);
        assert_eq!(score.q_count, 0.5);
    }

    #[test]
    fn q_count_floors_for_multi_token_rare_query() {
        let score = score_query("ACRE2 Jammer", Some(20_000), &CorpusStats::default());

        assert_eq!(score.q_count, 0.5);
        assert!(score.q >= MIN_Q);
    }

    #[test]
    fn q_count_unchanged_for_single_token() {
        let score = score_query("ACRE2", Some(20_000), &CorpusStats::default());

        assert_eq!(score.q_count, 0.0);
    }

    #[test]
    fn description_match_counts_as_corroboration() {
        let corroboration = Corroboration {
            description_match: true,
            ..Default::default()
        };

        assert!(corroboration.any());
    }

    #[test]
    fn decide_maps_description_match_to_matched_by() {
        let ranked = vec![RankedCandidate {
            id: "1".to_string(),
            title: "ADE - Advanced Diving Environment".to_string(),
            file_type: 0,
            m: 1.0,
            c: 1.0,
            head: 1.0,
            anchor_matched: true,
            corroboration: Corroboration {
                description_match: true,
                ..Default::default()
            },
        }];

        let decision = decide(
            &ranked,
            &query("Advanced Diving Environment", "diving"),
            &score("Advanced Diving Environment"),
            None,
        );

        assert_eq!(decision.status, ResolutionStatus::Confirmed);
        assert_eq!(decision.matched_by, ["item_description"]);
    }

    #[test]
    fn decide_confirms_on_all_gates() {
        let evidence = evidence(Some("ADE"));
        let ranked = rank(
            &[item("1", "ADE - Advanced Diving Environment", 0)],
            &normalise("ADE diving"),
            "diving",
            &evidence,
            None,
        );

        let q = score("ADE diving");
        let decision = decide(&ranked, &query("ADE diving", "diving"), &q, None);

        assert_eq!(decision.status, ResolutionStatus::Confirmed);
        assert_eq!(
            decision.title.as_deref(),
            Some("ADE - Advanced Diving Environment")
        );
    }

    #[test]
    fn decide_ambiguous_on_small_margin() {
        let evidence = evidence(Some("ADE"));
        let ranked = rank(
            &[
                item("1", "ADE - Advanced Diving Environment", 0),
                item("2", "ADE - Advanced Diving Environment", 0),
            ],
            &normalise("ADE diving"),
            "diving",
            &evidence,
            None,
        );

        let decision = decide(
            &ranked,
            &query("ADE diving", "diving"),
            &score("ADE diving"),
            None,
        );

        assert_eq!(decision.status, ResolutionStatus::Ambiguous);
        assert!(decision.title.is_none());
    }

    #[test]
    fn decide_never_confirms_non_mod() {
        let evidence = evidence(Some("ADE"));
        let ranked = rank(
            &[item("1", "ADE - Advanced Diving Environment", 2)],
            &normalise("ADE diving"),
            "diving",
            &evidence,
            None,
        );

        let decision = decide(
            &ranked,
            &query("ADE diving", "diving"),
            &score("ADE diving"),
            None,
        );

        assert_ne!(decision.status, ResolutionStatus::Confirmed);
    }

    #[test]
    fn decide_confirms_on_workshop_url() {
        let ranked = rank(
            &[item("123", "Something Else", 0)],
            &normalise("ADE"),
            "ade",
            &evidence(None),
            None,
        );

        let decision = decide(
            &ranked,
            &query(
                "https://steamcommunity.com/sharedfiles/filedetails/?id=123",
                "123",
            ),
            &score("ADE"),
            Some("123"),
        );

        assert_eq!(decision.status, ResolutionStatus::Confirmed);
        assert_eq!(decision.matched_by, ["workshop_url"]);
        assert_eq!(decision.c, 1.0);
    }

    #[test]
    fn decide_confirms_on_three_rare_shared_tokens() {
        let evidence = evidence(None);
        let pool = EvidencePool {
            strings: ["Diving Mask", "Wetsuit", "Rebreather"]
                .iter()
                .map(|text| crate::investigate::evidence::EvidenceString {
                    pbo: evidence.pbo.clone(),
                    text: text.to_string(),
                    key: crate::investigate::normalise::normalise(text).text,
                    kind: crate::investigate::evidence::EvidenceKind::Quoted,
                    token_df: 1,
                    pbo_df: 1,
                })
                .collect(),
            corpus: CorpusStats::default(),
            truncated: false,
        };
        let ranked = rank(
            &[item("1", "Diving Mask Wetsuit Rebreather Set", 0)],
            &normalise("diving"),
            "diving",
            &evidence,
            Some(&pool),
        );

        let decision = decide(&ranked, &query("diving", "diving"), &score("diving"), None);

        assert_eq!(decision.status, ResolutionStatus::Confirmed);
    }

    #[test]
    fn evidence_gate_rejects_unanchored_title_at_anchor_cap() {
        let evidence = evidence(None);
        let pool = pool_of(
            &evidence.pbo,
            &[
                ("Diving Mask", EvidenceKind::Quoted),
                ("Wetsuit", EvidenceKind::Quoted),
                ("Rebreather", EvidenceKind::Quoted),
            ],
        );
        let ranked = rank(
            &[item("1", "Diving Mask Wetsuit Rebreather Set", 0)],
            &normalise("diving mask set"),
            "ade",
            &evidence,
            Some(&pool),
        );
        let best = &ranked[0];
        assert_eq!(best.m, ANCHOR_CAP);
        assert!(!best.anchor_matched);
        assert!(best.corroboration.evidence_gate());
        let q = score("diving mask set");
        assert!(q.q >= MIN_Q);

        let decision = decide(&ranked, &query("diving mask set", "ade"), &q, None);

        assert_ne!(decision.status, ResolutionStatus::Confirmed);
        assert!(decision.title.is_none());
    }

    #[test]
    fn evidence_gate_rejects_a_weak_title_even_with_content_overlap() {
        // Three rare shared tokens would confirm a title that named the
        // item. This title does not reach `MIN_M`, so content overlap
        // cannot carry it.
        let ranked = vec![RankedCandidate {
            id: "1".to_string(),
            title: "Diving Mask Wetsuit Set".to_string(),
            file_type: 0,
            m: 0.70,
            c: 0.70 * 0.83,
            head: 0.0,
            anchor_matched: true,
            corroboration: Corroboration {
                evidence_overlap: EVIDENCE_MIN_TOKENS,
                evidence_tokens: vec!["rebreather".to_string()],
                ..Default::default()
            },
        }];
        let best = &ranked[0];
        assert!(best.m >= ANCHOR_CAP);
        assert!(best.m < MIN_M);
        assert!(best.corroboration.evidence_gate());
        let q = score("diving mask set");
        assert!(q.q >= MIN_Q);

        let decision = decide(&ranked, &query("diving mask set", "diving"), &q, None);

        assert_ne!(decision.status, ResolutionStatus::Confirmed);
        assert!(decision.title.is_none());
    }

    #[test]
    fn decide_rejects_stopword_only_overlap() {
        let evidence = evidence(None);
        let pool = EvidencePool {
            strings: ["class config displayname"]
                .iter()
                .map(|text| crate::investigate::evidence::EvidenceString {
                    pbo: evidence.pbo.clone(),
                    text: text.to_string(),
                    key: crate::investigate::normalise::normalise(text).text,
                    kind: crate::investigate::evidence::EvidenceKind::Bare,
                    token_df: 1,
                    pbo_df: 1,
                })
                .collect(),
            corpus: CorpusStats::default(),
            truncated: false,
        };
        let ranked = rank(
            &[item("1", "Class Config Displayname", 0)],
            &normalise("ade"),
            "ade",
            &evidence,
            Some(&pool),
        );

        let decision = decide(&ranked, &query("ade", "ade"), &score("ade"), None);

        assert_ne!(decision.status, ResolutionStatus::Confirmed);
    }
}

#[cfg(test)]
mod accuracy_tests {
    use super::*;
    use crate::investigate::browse::BrowseResult;
    use crate::investigate::identity::PhraseSource;
    use crate::investigate::plan::QueryKind;

    fn item(id: &str, title: &str, subs: u64) -> BrowseResult {
        BrowseResult {
            id: id.to_string(),
            title: title.to_string(),
            short_description: String::new(),
            tags: Vec::new(),
            subscriptions: subs,
            lifetime_subscriptions: subs,
            views: subs,
            favorited: 0,
            time_created: 0,
            time_updated: 0,
            file_size: String::new(),
            num_children: 0,
            children: Vec::new(),
            star_rating: 0.0,
            total_votes: 0,
            creator: String::new(),
            file_type: 0,
        }
    }

    fn query(text: &str, anchor: &str) -> PlannedQuery {
        PlannedQuery {
            text: text.to_string(),
            kind: QueryKind::IdentityName,
            source: PhraseSource::CfgModsName,
            anchor: anchor.to_string(),
        }
    }

    #[test]
    fn rank_never_orders_by_popularity() {
        let evidence = IdentityEvidence {
            pbo: "x.pbo".to_string(),
            ..Default::default()
        };
        let ranked = rank(
            &[
                item("200", "Same Title", 900_000),
                item("100", "Same Title", 2),
            ],
            &normalise("same title"),
            "title",
            &evidence,
            None,
        );

        assert_eq!(ranked[0].id, "100");
        assert_eq!(ranked[0].m, ranked[1].m);
    }

    #[test]
    fn decide_evidence_never_breaks_a_tie() {
        let evidence = IdentityEvidence {
            pbo: "x.pbo".to_string(),
            ..Default::default()
        };
        let pool = EvidencePool {
            strings: ["Diving Mask", "Wetsuit", "Rebreather"]
                .iter()
                .map(|text| crate::investigate::evidence::EvidenceString {
                    pbo: evidence.pbo.clone(),
                    text: text.to_string(),
                    key: crate::investigate::normalise::normalise(text).text,
                    kind: crate::investigate::evidence::EvidenceKind::Quoted,
                    token_df: 1,
                    pbo_df: 1,
                })
                .collect(),
            corpus: CorpusStats::default(),
            truncated: false,
        };
        let ranked = rank(
            &[
                item("1", "Diving Mask Wetsuit Rebreather Set", 10),
                item("2", "Diving Mask Wetsuit Rebreather Set", 90_000),
            ],
            &normalise("diving"),
            "diving",
            &evidence,
            Some(&pool),
        );

        let decision = decide(
            &ranked,
            &query("diving", "diving"),
            &score_query("diving", Some(50), &CorpusStats::default()),
            None,
        );

        assert_ne!(decision.status, ResolutionStatus::Confirmed);
        assert!(decision.title.is_none());
    }
}

#[cfg(test)]
mod specificity_tests {
    use super::*;
    use crate::investigate::browse::BrowseResult;
    use crate::investigate::identity::PhraseSource;
    use crate::investigate::plan::QueryKind;

    fn item(id: &str, title: &str) -> BrowseResult {
        BrowseResult {
            id: id.to_string(),
            title: title.to_string(),
            short_description: "available off sog".to_string(),
            tags: Vec::new(),
            subscriptions: 10,
            lifetime_subscriptions: 10,
            views: 10,
            favorited: 0,
            time_created: 0,
            time_updated: 0,
            file_size: String::new(),
            num_children: 0,
            children: Vec::new(),
            star_rating: 0.0,
            total_votes: 0,
            creator: String::new(),
            file_type: 0,
        }
    }

    #[test]
    fn decide_evidence_requires_specific_query() {
        let evidence = IdentityEvidence {
            pbo: "cba_common.pbo".to_string(),
            ..Default::default()
        };
        let pool = EvidencePool {
            strings: ["available", "off", "sog"]
                .iter()
                .map(|text| crate::investigate::evidence::EvidenceString {
                    pbo: evidence.pbo.clone(),
                    text: text.to_string(),
                    key: crate::investigate::normalise::normalise(text).text,
                    kind: crate::investigate::evidence::EvidenceKind::Quoted,
                    token_df: 1,
                    pbo_df: 1,
                })
                .collect(),
            corpus: CorpusStats::default(),
            truncated: false,
        };
        let ranked = rank(
            &[item("1", "Gamenator's 40mm addons (SOG&CBA)")],
            &normalise("addons"),
            "addons",
            &evidence,
            Some(&pool),
        );
        let query = PlannedQuery {
            text: "addons".to_string(),
            kind: QueryKind::IdentityName,
            source: PhraseSource::Stem,
            anchor: "addons".to_string(),
        };
        let q = score_query("addons", Some(3000), &CorpusStats::default());

        assert!(q.q < MIN_Q);
        let decision = decide(&ranked, &query, &q, None);

        assert_ne!(decision.status, ResolutionStatus::Confirmed);
    }
}

#[cfg(test)]
mod shared_title_tests {
    use super::*;
    use crate::investigate::browse::BrowseResult;
    use crate::investigate::identity::PhraseSource;
    use crate::investigate::plan::QueryKind;

    fn item(id: &str, title: &str) -> BrowseResult {
        BrowseResult {
            id: id.to_string(),
            title: title.to_string(),
            short_description: String::new(),
            tags: Vec::new(),
            subscriptions: 0,
            lifetime_subscriptions: 0,
            views: 0,
            favorited: 0,
            time_created: 0,
            time_updated: 0,
            file_size: String::new(),
            num_children: 0,
            children: Vec::new(),
            star_rating: 0.0,
            total_votes: 0,
            creator: String::new(),
            file_type: 0,
        }
    }

    #[test]
    fn ambiguous_reports_shared_title() {
        let evidence = IdentityEvidence {
            pbo: "ade_core.pbo".to_string(),
            ..Default::default()
        };
        let ranked = rank(
            &[
                item("1", "ADE - Advanced Diving Environment"),
                item(
                    "2",
                    "ADE - Advanced Diving Environment (Working Keys | 2020)",
                ),
                item("3", "Advanced Diving Environment (KAT)"),
                item("4", "ADE - Advanced Diving Environment - Aegis"),
                item("5", "[A.D.E.] Advanced Diving Environment - Reloaded"),
            ],
            &normalise("Advanced Diving Environment"),
            "diving",
            &evidence,
            None,
        );
        let query = PlannedQuery {
            text: "Advanced Diving Environment".to_string(),
            kind: QueryKind::IdentityName,
            source: PhraseSource::CbaOptionName,
            anchor: "diving".to_string(),
        };
        let q = score_query(
            "Advanced Diving Environment",
            Some(200),
            &CorpusStats::default(),
        );

        let decision = decide(&ranked, &query, &q, None);

        assert_eq!(decision.status, ResolutionStatus::Ambiguous);
        assert!(decision.title.is_none());
        assert!(decision
            .matched_by
            .iter()
            .any(|signal| signal == "shared_title:Advanced Diving Environment"));
    }
}

#[cfg(test)]
mod brand_tests {
    use super::*;
    use crate::investigate::browse::BrowseResult;

    fn item(title: &str) -> BrowseResult {
        BrowseResult {
            id: "2333743487".to_string(),
            title: title.to_string(),
            short_description: String::new(),
            tags: Vec::new(),
            subscriptions: 0,
            lifetime_subscriptions: 0,
            views: 0,
            favorited: 0,
            time_created: 0,
            time_updated: 0,
            file_size: String::new(),
            num_children: 0,
            children: Vec::new(),
            star_rating: 0.0,
            total_votes: 0,
            creator: String::new(),
            file_type: 0,
        }
    }

    #[test]
    fn brand_suffix_matches_rksl() {
        let score = score_match(
            &item("RKSL Studios Common Library"),
            &normalise("rksla3 common"),
            "rksla3",
        );

        assert_eq!(score.m, 1.0);
        assert!(score.anchor_matched);
    }

    #[test]
    fn brand_suffix_rejects_long_difference() {
        let score = score_match(&item("RKSL"), &normalise("rkslstudios"), "rkslstudios");

        assert!(score.m < 0.40);
    }
}
