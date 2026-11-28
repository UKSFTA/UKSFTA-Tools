//! One resolver: PBO evidence in, one decision per PBO out.
//!
//! Queries are memoised across a whole run, so two PBOs that plan the
//! same text cost one request. A cluster is only a request optimisation;
//! every PBO still gets its own decision.
//!
//! Three rules keep repeated runs useful. A record the store already
//! answers is reused instead of asked again. The remaining clusters are
//! ordered by how long ago they were tried, so a run stopped by the
//! operator hands the next run a different set. And a candidate that two
//! distinct queries both rank near the top is confirmed as agreement,
//! but only when something in the PBO corroborates it.
//!
//! A first page that comes back empty is fetched once more after a pause.
//! Steam sometimes answers a search with an empty page while it throttles,
//! and a second paced request tells that apart from a real absence.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::time::Duration;

use super::author::{author_matches, fetch_item_page, has_usable_author, ItemPage};
use super::browse::{search_browse_page, BrowsePage, BrowseResult, SearchError};
use super::cluster::{cluster_by_identity, Cluster};
use super::evidence::EvidencePool;
use super::http::{Endpoint, Http, RateLimiter};
use super::identity::{IdentityEvidence, PhraseSource};
use super::keyed::{merge_ranked, KeyedError, KeyedSearch};
use super::normalise::normalise;
use super::plan::{
    is_generic_word, is_structural_stop, plan_queries, CorpusStats, PlannedQuery, PlannerContext,
    QueryKind,
};
use super::r#match::{
    decide, rank, score_query, QueryScore, RankedCandidate, Resolution, ResolutionStatus, MIN_M,
    MIN_MARGIN, MIN_Q,
};
use super::search::search;
use super::store::{attempted_at, entry_is_fresh, CandidateRecord, IdentityStore};

/// Queries tried per cluster before the evidence is treated as too weak.
const MAX_QUERIES: usize = 3;

/// Ranked candidates recorded per PBO, so a report can measure precision
/// at rank without a second run.
const CANDIDATE_TOP: usize = 10;

/// Tied candidates a content pass may read. Each costs one item page, so
/// the pass is bounded before any page is fetched.
const TIE_TOP: usize = 5;

/// The shortest PBO content token that can name an addon. Below it a word
/// is a join word or an acronym the description uses for anything.
const CONTENT_TOKEN_MIN_LEN: usize = 6;

/// Ranked candidates that count towards cross-query agreement.
const CROSS_QUERY_TOP: usize = 5;

/// The match score an agreeing candidate must reach. Below it the title
/// only brushed the query, and a rank position means nothing.
const CROSS_QUERY_MIN_M: f64 = 0.40;

/// Distinct queries that must agree before a candidate is confirmed.
const CROSS_QUERY_MIN_QUERIES: usize = 2;

/// One request and what came back.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueryAttempt {
    pub text: String,
    pub kind: QueryKind,
    pub q: f64,
    pub total_count: Option<u64>,
    pub outcome: ResolutionStatus,
}

/// One candidate a search returned, with the size of that result set.
#[derive(Debug, Clone)]
pub(crate) struct SeenItem {
    pub item: BrowseResult,
    pub total_count: u32,
}

/// One step of a run, for a caller that shows live progress.
pub(crate) struct Progress {
    pub done: usize,
    pub total: usize,
    pub label: String,
}

/// The sink a caller installs to receive each [`Progress`] step.
type ProgressSink<'a> = Box<dyn FnMut(&Progress) + 'a>;

/// An optional content index that maps a PBO fingerprint to the Workshop
/// items its manifest names. The manifest index lands separately; until
/// then the resolver holds no index, and a candidate confirms only through
/// the evidence rules.
pub(crate) trait ContentIndex {
    fn confirms(&self, fingerprint: &str, item: &str) -> bool;
}

/// The keyed search's view of one PBO: the queries sent and the ranked
/// candidates they returned. It is a report only; the decision is in
/// [`PboResolution::resolution`].
#[derive(Debug, Clone)]
pub(crate) struct KeyedReport {
    pub pbo: String,
    pub queries: Vec<String>,
    pub ranked: Vec<(String, String)>,
}

/// Everything the tool decided about one PBO.
#[derive(Debug, Clone)]
pub(crate) struct PboResolution {
    pub pbo: String,
    pub fingerprint: String,
    pub evidence: IdentityEvidence,
    pub attempts: Vec<QueryAttempt>,
    pub resolution: Resolution,
    /// The strongest candidates the queries ranked, best score first. Empty
    /// when no query was sent for this PBO.
    pub candidates: Vec<CandidateRecord>,
}

impl PboResolution {
    fn without_query(evidence: &IdentityEvidence) -> Self {
        Self {
            pbo: evidence.pbo.clone(),
            fingerprint: evidence.fingerprint.clone(),
            evidence: evidence.clone(),
            attempts: Vec::new(),
            resolution: Resolution {
                status: ResolutionStatus::NoDistinctiveQuery,
                id: None,
                title: None,
                m: 0.0,
                q: 0.0,
                c: 0.0,
                margin: 0.0,
                query: None,
                matched_by: Vec::new(),
            },
            candidates: Vec::new(),
        }
    }
}

/// Shared inputs for every lookup in one run.
pub(crate) struct ResolveContext<'a> {
    pub http: &'a dyn Http,
    pub limiter: &'a mut RateLimiter,
    pub corpus: &'a CorpusStats,
    pub pool: Option<&'a EvidencePool>,
    /// Query text to outcome. One memo per run, so two clusters that plan
    /// the same text cost one request.
    memo: HashMap<String, Memo>,
    /// The store the caller loaded before resolving. Scheduling reads the
    /// attempt times in it; the reuse decision itself happens in
    /// [`split_fresh`].
    store: Option<&'a IdentityStore>,
    /// Item pages already fetched in this run, keyed by item id. One item
    /// costs one request however many PBOs point at it.
    page_memo: HashMap<String, Option<ItemPage>>,
    /// Called once before the cluster loop and once per resolved cluster.
    progress: Option<ProgressSink<'a>>,
    /// The opt-in keyed search. Without it this resolver behaves exactly
    /// as it did before the flag existed.
    keyed: Option<&'a dyn KeyedSearch>,
    /// Query text to its keyed results, so a shared query costs one call.
    keyed_pages: HashMap<String, Vec<BrowseResult>>,
    /// Query texts already sent to the keyed source.
    keyed_tried: HashSet<String>,
    /// A refused key. It stops the run once the caller reads it.
    keyed_failure: Option<KeyedError>,
    /// The keyed ranking per PBO, for the search report.
    keyed_report: BTreeMap<String, KeyedReport>,
    /// The optional content-hash index. No implementation exists yet.
    pub(crate) content_index: Option<&'a dyn ContentIndex>,
    /// An item id that must never become a candidate: the pack under
    /// investigation is not a source for its own files.
    excluded: Option<String>,
}

impl<'a> ResolveContext<'a> {
    pub(crate) fn new(
        http: &'a dyn Http,
        limiter: &'a mut RateLimiter,
        corpus: &'a CorpusStats,
        pool: Option<&'a EvidencePool>,
    ) -> Self {
        Self {
            http,
            limiter,
            corpus,
            pool,
            memo: HashMap::new(),
            store: None,
            page_memo: HashMap::new(),
            progress: None,
            keyed: None,
            keyed_pages: HashMap::new(),
            keyed_tried: HashSet::new(),
            keyed_failure: None,
            keyed_report: BTreeMap::new(),
            content_index: None,
            excluded: None,
        }
    }

    /// Add the keyed search as a candidate source. Without this call the
    /// resolver sends only the browse request it always sent.
    pub(crate) fn use_keyed_search(&mut self, source: &'a dyn KeyedSearch) {
        self.keyed = Some(source);
    }

    /// The refused-key error, if the keyed source rejected the key.
    pub(crate) fn keyed_failure(&self) -> Option<&KeyedError> {
        self.keyed_failure.as_ref()
    }

    /// The keyed ranking per PBO, sorted by PBO name. Empty without the
    /// keyed source.
    pub(crate) fn keyed_report(&self) -> Vec<KeyedReport> {
        self.keyed_report.values().cloned().collect()
    }

    fn keyed_enabled(&self) -> bool {
        self.keyed.is_some()
    }

    /// Send progress to `sink`. Without a sink a run reports nothing.
    pub(crate) fn set_progress(&mut self, sink: ProgressSink<'a>) {
        self.progress = Some(sink);
    }

    /// Report one step. A run with no sink does nothing.
    fn report_progress(&mut self, done: usize, total: usize, label: String) {
        if let Some(sink) = self.progress.as_mut() {
            sink(&Progress { done, total, label });
        }
    }

    /// Schedule the clusters from the store's attempt times. Without a
    /// store every cluster is equally old and the key alone orders them.
    pub(crate) fn schedule_from(&mut self, store: &'a IdentityStore) {
        self.store = Some(store);
    }

    /// One item page, from the memo or from the item itself.
    ///
    /// A failed fetch is recorded as absent, so a second PBO that points
    /// at the same item does not ask again in this run.
    fn item_page(&mut self, id: &str) -> Option<&ItemPage> {
        if !self.page_memo.contains_key(id) {
            let page = fetch_item_page(self.http, &mut *self.limiter, id);
            self.page_memo.insert(id.to_string(), page);
        }
        self.page_memo.get(id).and_then(Option::as_ref)
    }

    /// The creator name of one item, from the memo or from its page.
    fn item_author(&mut self, id: &str) -> Option<String> {
        self.item_page(id).and_then(|page| page.author.clone())
    }

    /// Never treat this Workshop id as a candidate.
    pub(crate) fn exclude_item(&mut self, id: String) {
        self.excluded = Some(id);
    }

    /// Drop the excluded id from one page of results.
    fn drop_excluded(&self, results: &mut Vec<BrowseResult>) {
        if let Some(excluded) = self.excluded.as_deref() {
            results.retain(|result| result.id != excluded);
        }
    }

    /// The keyed results for one query, memoised by text. A refused key is
    /// recorded once and stops every later request, so a bad key costs one
    /// call, not one per PBO.
    fn keyed_page(&mut self, query: &str) -> Option<Vec<BrowseResult>> {
        let source = self.keyed?;
        if self.keyed_failure.is_some() {
            return None;
        }
        if !self.keyed_tried.contains(query) {
            self.limiter.wait(Endpoint::QueryFiles);
            match source.query(query) {
                Ok(page) => {
                    let mut results = page.results;
                    self.drop_excluded(&mut results);
                    self.keyed_pages.insert(query.to_string(), results);
                }
                Err(error) if error.is_fatal() => self.keyed_failure = Some(error),
                Err(error) => {
                    eprintln!("  steam search failed: \"{query}\" ({})", error.message());
                    self.keyed_pages.insert(query.to_string(), Vec::new());
                }
            }
            self.keyed_tried.insert(query.to_string());
        }
        self.keyed_pages.get(query).cloned()
    }

    /// Every distinct candidate this run saw, sorted by id. Feeds the item
    /// cache only; no decision reads it, so the collection order cannot
    /// reach a score.
    pub(crate) fn seen_items(&self) -> Vec<SeenItem> {
        let mut items: BTreeMap<&str, SeenItem> = BTreeMap::new();
        for memo in self.memo.values() {
            let Memo::Page(page) = memo else {
                continue;
            };
            for result in &page.results {
                let total_count = page.total_count.min(u64::from(u32::MAX)) as u32;
                match items.get(result.id.as_str()) {
                    // A duplicate id keeps the higher subscription count, then
                    // the larger title, so the choice never depends on the
                    // HashMap iteration order above.
                    Some(existing)
                        if (existing.item.subscriptions, &existing.item.title)
                            >= (result.subscriptions, &result.title) => {}
                    _ => {
                        items.insert(
                            result.id.as_str(),
                            SeenItem {
                                item: result.clone(),
                                total_count,
                            },
                        );
                    }
                }
            }
        }
        items.into_values().collect()
    }
}

#[derive(Clone)]
enum Memo {
    Page(BrowsePage),
    Failed(ResolutionStatus),
}

impl Memo {
    fn outcome(&self) -> Result<BrowsePage, ResolutionStatus> {
        match self {
            Memo::Page(page) => Ok(page.clone()),
            Memo::Failed(status) => Err(*status),
        }
    }
}

/// Split evidence into records that may stand and PBOs that still need a
/// query.
///
/// A record stands only while its fingerprint matches the file on disk and
/// its status is not due a retry. A standing record is rendered from the
/// store and spends no request.
pub(crate) fn split_fresh(
    store: &IdentityStore,
    evidence: Vec<IdentityEvidence>,
    now: u64,
) -> (Vec<PboResolution>, Vec<IdentityEvidence>) {
    let mut kept = Vec::new();
    let mut pending = Vec::new();
    for item in evidence {
        match store.pbo.get(&item.pbo) {
            Some(entry) if entry_is_fresh(entry, &item.fingerprint, now) => {
                kept.push(entry.to_resolution(&item.pbo));
            }
            _ => pending.push(item),
        }
    }
    kept.sort_by(|left, right| left.pbo.cmp(&right.pbo));
    (kept, pending)
}

/// [`split_fresh`], or every PBO pending when the operator asked for a
/// refresh. A stored decision can be wrong, and it stands while the
/// fingerprint matches, so without this the only way past it is deleting
/// the store.
pub(crate) fn split_fresh_with(
    store: &IdentityStore,
    evidence: Vec<IdentityEvidence>,
    now: u64,
    refresh: bool,
) -> (Vec<PboResolution>, Vec<IdentityEvidence>) {
    if refresh {
        return (Vec::new(), evidence);
    }
    split_fresh(store, evidence, now)
}

/// Resolve every pending PBO. Exactly one result per input, sorted by PBO name.
pub(crate) fn resolve_all(
    evidence: Vec<IdentityEvidence>,
    ctx: &mut ResolveContext,
) -> Vec<PboResolution> {
    ctx.memo.clear();
    let mut clusters = cluster_by_identity(&evidence);
    // Least recently attempted first, then by cluster key, so a run that
    // the operator interrupts does not repeat the same head of the list.
    clusters.sort_by(|left, right| {
        (cluster_age(ctx.store, left), &left.key).cmp(&(cluster_age(ctx.store, right), &right.key))
    });
    let mut resolved: Vec<PboResolution> = Vec::with_capacity(evidence.len());
    let mut clustered: HashSet<&str> = HashSet::new();
    let total = clusters.len();
    let mut done = 0usize;
    ctx.report_progress(0, total, String::new());

    for cluster in &clusters {
        for member in &cluster.members {
            clustered.insert(member.as_str());
        }
        resolved.extend(resolve_cluster(cluster, ctx));
        done += 1;
        let label = cluster
            .members
            .first()
            .map(|pbo| pbo.strip_suffix(".pbo").unwrap_or(pbo).to_string())
            .unwrap_or_default();
        ctx.report_progress(done, total, label);
    }

    for item in &evidence {
        if !clustered.contains(item.pbo.as_str()) {
            resolved.push(PboResolution::without_query(item));
        }
    }

    resolved.sort_by(|left, right| left.pbo.cmp(&right.pbo));
    resolved
}

/// The oldest stored attempt among a cluster's members, or zero when none
/// of them was ever queried. A cluster shares one request, so its place in
/// the schedule is the place of its least recently attempted member.
fn cluster_age(store: Option<&IdentityStore>, cluster: &Cluster) -> u64 {
    let Some(store) = store else {
        return 0;
    };
    cluster
        .members
        .iter()
        .map(|pbo| attempted_at(store, pbo))
        .min()
        .unwrap_or(0)
}

/// Resolve every PBO that shares one cluster phrase with one request set.
///
/// The plan is built once for the cluster, so the strongest phrase is
/// searched once whatever the number of members. Each member still gets
/// its own ranking, corroboration and decision. A cluster no query settled
/// spends one more request on page two of its best query, then reads the
/// item pages of any remaining tie.
pub(crate) fn resolve_cluster(cluster: &Cluster, ctx: &mut ResolveContext) -> Vec<PboResolution> {
    let mut resolved: Vec<PboResolution> = cluster
        .evidence
        .iter()
        .map(PboResolution::without_query)
        .collect();
    let Some(leader) = cluster.evidence.first() else {
        return resolved;
    };

    let terms = group_terms(&cluster.evidence);
    let plan = plan_queries(
        leader,
        &PlannerContext {
            corpus: ctx.corpus,
            group_terms: &terms,
            pool: ctx.pool,
        },
    );
    if plan.queries.is_empty() {
        return resolved;
    }

    let pool = ctx.pool;
    let mut agreement: Vec<BTreeMap<String, Agreement>> = vec![BTreeMap::new(); resolved.len()];
    let mut candidate_maps: Vec<BTreeMap<String, CandidateRecord>> =
        vec![BTreeMap::new(); resolved.len()];
    let mut ties: Vec<Option<TieContext>> = (0..resolved.len()).map(|_| None).collect();
    let mut best: Option<BestPage> = None;
    let mut keyed_pages: Vec<BrowsePage> = Vec::new();
    let mut keyed_queries: Vec<String> = Vec::new();

    for query in plan.queries.iter().take(MAX_QUERIES) {
        let page = match memoised(ctx, &query.text) {
            Ok(page) => page,
            Err(status) => {
                for item in &mut resolved {
                    item.attempts.push(QueryAttempt {
                        text: query.text.clone(),
                        kind: query.kind,
                        q: 0.0,
                        total_count: None,
                        outcome: status,
                    });
                    item.resolution = failed(status, &query.text);
                }
                if ctx.keyed_enabled() {
                    keyed_queries.push(query.text.clone());
                }
                if status == ResolutionStatus::RateLimited {
                    break;
                }
                continue;
            }
        };

        // The memo already holds this query's keyed results, so the report
        // costs no second request.
        if ctx.keyed_enabled() {
            keyed_queries.push(query.text.clone());
            if let Some(results) = ctx.keyed_page(&query.text) {
                if !results.is_empty() {
                    keyed_pages.push(keyed_only_page(results));
                }
            }
        }

        let phrase = normalise(&query.text);
        let query_score = score_query(&query.text, Some(page.total_count), ctx.corpus);
        if best
            .as_ref()
            .is_none_or(|held| query_score.q > held.score.q)
        {
            best = Some(BestPage {
                query: query.clone(),
                score: query_score,
                page: page.clone(),
            });
        }
        let mut confirmed = false;

        for (index, ((item, seen), candidates)) in resolved
            .iter_mut()
            .zip(agreement.iter_mut())
            .zip(candidate_maps.iter_mut())
            .enumerate()
        {
            let ranked = rank(&page.results, &phrase, &query.anchor, &item.evidence, pool);
            record_agreement(seen, &ranked, &query.text, query_score.q);
            record_candidates(candidates, &ranked);
            let url_id = workshop_url_id(&item.evidence);
            let mut decision = decide(&ranked, query, &query_score, url_id.as_deref());
            if let Some(promoted) = author_promotion(
                ctx,
                &item.evidence,
                &ranked,
                query,
                &query_score,
                url_id.as_deref(),
            ) {
                decision = promoted;
            }
            prune_shared_title(&mut decision, &item.evidence);
            confirmed |= decision.status == ResolutionStatus::Confirmed;
            let first_attempt = item.attempts.is_empty();
            item.attempts.push(QueryAttempt {
                text: query.text.clone(),
                kind: query.kind,
                q: query_score.q,
                total_count: Some(page.total_count),
                outcome: decision.status,
            });
            if first_attempt
                || decision.status == ResolutionStatus::Confirmed
                || item.resolution.status == ResolutionStatus::Unresolved
                || decision.m > item.resolution.m
            {
                settle(
                    &mut item.resolution,
                    &mut ties[index],
                    decision,
                    ranked,
                    query,
                    &query_score,
                    url_id,
                );
            }
        }

        if confirmed {
            break;
        }
    }

    apply_agreement(&mut resolved, &agreement);

    let nothing_confirmed = resolved
        .iter()
        .all(|item| item.resolution.status != ResolutionStatus::Confirmed);
    if nothing_confirmed {
        if let Some(best) = best.filter(|held| held.score.q >= MIN_Q) {
            page_two(
                ctx,
                &best,
                &mut resolved,
                &mut agreement,
                &mut candidate_maps,
                &mut ties,
                pool,
            );
            apply_agreement(&mut resolved, &agreement);
        }
    }

    for (index, item) in resolved.iter_mut().enumerate() {
        if !tie_ready(&item.resolution) {
            continue;
        }
        let Some(tie) = ties[index].take() else {
            continue;
        };
        if let Some(confirmed) = tie_confirmation(ctx, &item.evidence, &tie) {
            item.resolution = confirmed;
        }
    }

    for (item, seen) in resolved.iter_mut().zip(candidate_maps) {
        item.candidates = top_candidates(seen);
    }

    if let Some(index) = ctx.content_index {
        apply_hash_index(index, &mut resolved);
    }

    if !keyed_queries.is_empty() {
        let ranked: Vec<(String, String)> = merge_ranked(&keyed_pages)
            .into_iter()
            .map(|item| (item.id, item.title))
            .collect();
        for item in &resolved {
            ctx.keyed_report.insert(
                item.pbo.clone(),
                KeyedReport {
                    pbo: item.pbo.clone(),
                    queries: keyed_queries.clone(),
                    ranked: ranked.clone(),
                },
            );
        }
    }

    resolved
}

/// The best page the cluster saw: the query with the highest Q that
/// returned one. Page two is worth asking for only of that query.
struct BestPage {
    query: PlannedQuery,
    score: QueryScore,
    page: BrowsePage,
}

/// One undecided line and the ranked list it was decided from.
struct TieContext {
    ranked: Vec<RankedCandidate>,
    query: PlannedQuery,
    score: QueryScore,
    url_id: Option<String>,
}

/// Record a decision and keep the list behind it for a later content pass.
fn settle(
    resolution: &mut Resolution,
    tie: &mut Option<TieContext>,
    decision: Resolution,
    ranked: Vec<RankedCandidate>,
    query: &PlannedQuery,
    score: &QueryScore,
    url_id: Option<String>,
) {
    let decided = decision.status == ResolutionStatus::Confirmed;
    *resolution = decision;
    *tie = (!decided).then(|| TieContext {
        ranked,
        query: query.clone(),
        score: *score,
        url_id,
    });
}

/// Confirm every line that independent queries already agreed on.
fn apply_agreement(resolved: &mut [PboResolution], agreement: &[BTreeMap<String, Agreement>]) {
    for (item, seen) in resolved.iter_mut().zip(agreement) {
        if item.resolution.status == ResolutionStatus::Confirmed {
            continue;
        }
        if let Some((id, hit)) = unique_agreement(seen) {
            item.resolution = agreed(id, hit, seen);
        }
    }
}

/// Merge page two of the best query into the decision, once per member.
///
/// Page one ended with nothing confirmed, so the deeper results are the
/// only untried evidence. A failed page leaves the earlier decision
/// standing.
fn page_two(
    ctx: &mut ResolveContext,
    best: &BestPage,
    resolved: &mut [PboResolution],
    agreement: &mut [BTreeMap<String, Agreement>],
    candidate_maps: &mut [BTreeMap<String, CandidateRecord>],
    ties: &mut [Option<TieContext>],
    pool: Option<&EvidencePool>,
) {
    let Ok(second) = memoised_page(ctx, &best.query.text, 2) else {
        return;
    };
    let merged = merge_results(&best.page, &second);
    if merged.is_empty() {
        return;
    }
    let phrase = normalise(&best.query.text);
    let total_count = Some(merged.len() as u64);

    for (index, ((item, seen), candidates)) in resolved
        .iter_mut()
        .zip(agreement.iter_mut())
        .zip(candidate_maps.iter_mut())
        .enumerate()
    {
        let ranked = dedupe_ranked(rank(
            &merged,
            &phrase,
            &best.query.anchor,
            &item.evidence,
            pool,
        ));
        record_agreement(seen, &ranked, &best.query.text, best.score.q);
        record_candidates(candidates, &ranked);
        let url_id = workshop_url_id(&item.evidence);
        let mut decision = decide(&ranked, &best.query, &best.score, url_id.as_deref());
        if let Some(promoted) = author_promotion(
            ctx,
            &item.evidence,
            &ranked,
            &best.query,
            &best.score,
            url_id.as_deref(),
        ) {
            decision = promoted;
        }
        prune_shared_title(&mut decision, &item.evidence);
        item.attempts.push(QueryAttempt {
            text: best.query.text.clone(),
            kind: best.query.kind,
            q: best.score.q,
            total_count,
            outcome: decision.status,
        });
        if decision.status == ResolutionStatus::Confirmed
            || item.resolution.status == ResolutionStatus::Unresolved
            || decision.m > item.resolution.m
        {
            settle(
                &mut item.resolution,
                &mut ties[index],
                decision,
                ranked,
                &best.query,
                &best.score,
                url_id,
            );
        }
    }
}

/// Page two's results after page one, without repeating an id. A page can
/// list an item the other already held.
fn merge_results(first: &BrowsePage, second: &BrowsePage) -> Vec<BrowseResult> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut merged: Vec<BrowseResult> =
        Vec::with_capacity(first.results.len() + second.results.len());
    for result in first.results.iter().chain(second.results.iter()) {
        if seen.insert(result.id.as_str()) {
            merged.push(result.clone());
        }
    }
    merged
}

/// Keep the best-scoring copy of an id. A duplicate would otherwise be its
/// own runner-up and erase the margin.
fn dedupe_ranked(ranked: Vec<RankedCandidate>) -> Vec<RankedCandidate> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    ranked
        .into_iter()
        .filter(|candidate| seen.insert(candidate.id.clone()))
        .collect()
}

/// Whether a line is a tie page content could still break: a strong title
/// on a specific query, held level with another candidate.
fn tie_ready(resolution: &Resolution) -> bool {
    resolution.status == ResolutionStatus::Ambiguous
        && resolution.m >= MIN_M
        && resolution.q >= MIN_Q
        && resolution.margin < MIN_MARGIN
}

/// Confirm one tied candidate from what its own page says.
///
/// The content words come from the PBO, so a match names the addon rather
/// than the search text. Exactly one candidate must answer the description,
/// else exactly one the author. Anything else leaves the tie standing and
/// the shared title on the line.
fn tie_confirmation(
    ctx: &mut ResolveContext,
    evidence: &IdentityEvidence,
    tie: &TieContext,
) -> Option<Resolution> {
    let tied = tied_candidates(&tie.ranked);
    if tied.len() < 2 {
        return None;
    }
    let tokens = content_tokens(evidence);
    let mut described: Vec<&RankedCandidate> = Vec::new();
    let mut authored: Vec<&RankedCandidate> = Vec::new();
    for candidate in &tied {
        let (by_description, by_author) = match ctx.item_page(&candidate.id) {
            Some(page) => (
                description_has(&page.description, &tokens),
                page.author
                    .as_deref()
                    .is_some_and(|author| author_matches(evidence, author)),
            ),
            None => (false, false),
        };
        if by_description {
            described.push(candidate);
        }
        if by_author {
            authored.push(candidate);
        }
    }

    match (described.as_slice(), authored.as_slice()) {
        ([candidate], _) if discriminates(candidate, &tied, &tokens) => {
            Some(confirm_tied(candidate, TieSignal::Description, tie))
        }
        (_, [candidate]) if discriminates(candidate, &tied, &tokens) => {
            Some(confirm_tied(candidate, TieSignal::Author, tie))
        }
        _ => None,
    }
}

/// Whether a PBO content token separates this candidate from the others.
/// A word every tied title carries ("tattoos", "compat") proves nothing.
fn discriminates(
    candidate: &RankedCandidate,
    tied: &[&RankedCandidate],
    tokens: &BTreeSet<String>,
) -> bool {
    let own: BTreeSet<String> = normalise(&candidate.title).tokens.into_iter().collect();
    tokens.iter().any(|token| {
        own.contains(token)
            && tied
                .iter()
                .filter(|other| other.id != candidate.id)
                .all(|other| {
                    !normalise(&other.title)
                        .tokens
                        .iter()
                        .any(|word| word == token)
                })
    })
}

/// Candidates level with the best score, best first, at most [`TIE_TOP`].
fn tied_candidates(ranked: &[RankedCandidate]) -> Vec<&RankedCandidate> {
    let Some(best) = ranked.first() else {
        return Vec::new();
    };
    ranked
        .iter()
        .filter(|candidate| level_with(candidate, best.m))
        .take(TIE_TOP)
        .collect()
}

/// How many results share the best score. Two or more is a tie no single
/// author can break.
fn tied_with_best(ranked: &[RankedCandidate]) -> usize {
    let Some(best) = ranked.first() else {
        return 0;
    };
    ranked
        .iter()
        .filter(|candidate| level_with(candidate, best.m))
        .count()
}

/// Whether a result could become a mod name and holds the best score.
fn level_with(candidate: &RankedCandidate, best: f64) -> bool {
    candidate.file_type == 0 && (candidate.m - best).abs() < f64::EPSILON
}

/// The PBO's own content words long enough to be a name rather than a join
/// word: the prefix, the CfgPatches class and the file stem.
fn content_tokens(evidence: &IdentityEvidence) -> BTreeSet<String> {
    let mut sources: Vec<&str> = evidence.prefix.as_deref().into_iter().collect();
    sources.extend(
        evidence
            .phrases
            .iter()
            .filter(|phrase| phrase.source == PhraseSource::CfgPatchesClass)
            .map(|phrase| phrase.text.as_str()),
    );
    sources.push(evidence.stem.as_str());
    sources
        .into_iter()
        .flat_map(|text| normalise(text).tokens)
        .filter(|token| {
            token.chars().count() >= CONTENT_TOKEN_MIN_LEN
                && !crate::investigate::plan::is_generic_word(token)
                && !crate::investigate::plan::is_structural_stop(token)
        })
        .collect()
}

/// Whether the item description carries one of the PBO's content words.
fn description_has(description: &str, tokens: &BTreeSet<String>) -> bool {
    if tokens.is_empty() || description.is_empty() {
        return false;
    }
    normalise(description)
        .tokens
        .iter()
        .any(|word| tokens.contains(word))
}

/// The item-page evidence that broke a tie.
#[derive(Debug, Clone, Copy)]
enum TieSignal {
    Description,
    Author,
}

impl TieSignal {
    const fn matched_by(self) -> &'static str {
        match self {
            TieSignal::Description => "item_description",
            TieSignal::Author => "item_author",
        }
    }
}

/// Confirm one tied candidate from the signal its page supplied.
///
/// `decide` runs on that candidate alone, because the page is what broke
/// the tie. The reported margin stays the real gap to the runner-up, so a
/// confirmed tie line never claims separation it did not have.
fn confirm_tied(candidate: &RankedCandidate, signal: TieSignal, tie: &TieContext) -> Resolution {
    let mut chosen = candidate.clone();
    match signal {
        TieSignal::Description => chosen.corroboration.description_match = true,
        TieSignal::Author => chosen.corroboration.author_match = true,
    }
    let mut resolution = decide(&[chosen], &tie.query, &tie.score, tie.url_id.as_deref());
    if resolution.status == ResolutionStatus::Confirmed {
        resolution.matched_by = vec![signal.matched_by().to_string()];
    }
    resolution.margin = margin_of(&tie.ranked);
    resolution
}

/// The gap between the best score and the next candidate. Zero when
/// nothing else was ranked.
fn margin_of(ranked: &[RankedCandidate]) -> f64 {
    let best = ranked.first().map(|candidate| candidate.m).unwrap_or(0.0);
    let second = ranked.get(1).map(|candidate| candidate.m).unwrap_or(0.0);
    (best - second).max(0.0)
}

/// The best candidate when every title gate passes on its own and only
/// corroboration is missing.
///
/// A tie is included: the page may still be read, and the content pass
/// decides whether one page's evidence separates the candidates. A
/// candidate that fails a gate, or a result set with no separation, is
/// never offered.
fn awaiting_corroboration<'r>(
    ranked: &'r [RankedCandidate],
    q: &QueryScore,
) -> Option<&'r RankedCandidate> {
    let best = ranked.first()?;
    (best.file_type == 0
        && best.m >= MIN_M
        && best.head >= 1.0
        && q.q >= MIN_Q
        && !best.corroboration.any())
    .then_some(best)
}

/// Decide again with the item page's creator as corroboration.
///
/// Returns `None` unless the PBO names an author, the best candidate already
/// passes every title gate, and the creator matches. The author promotes a
/// candidate the titles earned. It never rescues one they rejected, and it
/// never breaks a tie: two candidates at the same score wait for the content
/// pass, which reads every tied page before it confirms one.
fn author_promotion(
    ctx: &mut ResolveContext,
    evidence: &IdentityEvidence,
    ranked: &[RankedCandidate],
    query: &PlannedQuery,
    q: &QueryScore,
    url_id: Option<&str>,
) -> Option<Resolution> {
    let candidate = awaiting_corroboration(ranked, q)?;
    if !has_usable_author(evidence) {
        return None;
    }
    let author = ctx.item_author(&candidate.id)?;
    if !author_matches(evidence, &author) || tied_with_best(ranked) > 1 {
        return None;
    }
    let mut chosen = candidate.clone();
    chosen.corroboration.author_match = true;
    let mut resolution = decide(&[chosen], query, q, url_id);
    if resolution.status != ResolutionStatus::Confirmed {
        return None;
    }
    resolution.margin = margin_of(ranked);
    Some(resolution)
}

/// One candidate that at least one query ranked near the top.
#[derive(Debug, Clone)]
struct Agreement {
    title: String,
    m: f64,
    q: f64,
    /// The distinct query texts that ranked it. A repeated text counts once.
    queries: BTreeSet<String>,
    /// Whether any query that ranked it found the anchor token in the title.
    anchor_matched: bool,
    /// Whether the item is tied to this PBO by provenance rather than by a
    /// shared word.
    corroborated: bool,
}

/// Note every candidate this query placed in its top five with a real title
/// match. Agreement is a count of distinct queries, never of rank strength
/// or popularity.
fn record_agreement(
    seen: &mut BTreeMap<String, Agreement>,
    ranked: &[RankedCandidate],
    query: &str,
    q: f64,
) {
    for candidate in ranked.iter().take(CROSS_QUERY_TOP) {
        // A non-mod result can never become a mod name, however often it
        // ranks, and a title that only brushed the query proves nothing.
        if candidate.file_type != 0 || candidate.m < CROSS_QUERY_MIN_M {
            continue;
        }
        let entry = seen
            .entry(candidate.id.clone())
            .or_insert_with(|| Agreement {
                title: candidate.title.clone(),
                m: candidate.m,
                q,
                queries: BTreeSet::new(),
                anchor_matched: candidate.anchor_matched,
                corroborated: provenance_link(&candidate.corroboration),
            });
        entry.corroborated |= provenance_link(&candidate.corroboration);
        entry.anchor_matched |= candidate.anchor_matched;
        entry.queries.insert(query.to_string());
        entry.q = entry.q.max(q);
        if candidate.m > entry.m {
            entry.m = candidate.m;
            entry.title = candidate.title.clone();
        }
    }
}

/// Corroboration that can carry an agreement between queries. An uploader
/// name or a page description is not enough on its own: one author can
/// publish several compat mods with near-identical titles.
fn provenance_link(corroboration: &crate::investigate::r#match::Corroboration) -> bool {
    // A shared word proves nothing at a tie: ten titles contain "markers",
    // and a PBO stem of "Backpacks" appears in any backpack description.
    // Only a Workshop URL, a dependency root, the uploader, or the item
    // page itself ties the item to this file.
    corroboration.url_id_match
        || corroboration.dep_match
        || corroboration.author_match
        || corroboration.description_match
}

/// Keep the strongest score seen for each candidate id. A later query may
/// rank the same id lower, and the best score is the one the line reports.
fn record_candidates(seen: &mut BTreeMap<String, CandidateRecord>, ranked: &[RankedCandidate]) {
    for candidate in ranked {
        let better = seen
            .get(&candidate.id)
            .is_none_or(|existing| candidate.m > existing.m);
        if better {
            seen.insert(
                candidate.id.clone(),
                CandidateRecord {
                    id: candidate.id.clone(),
                    title: candidate.title.clone(),
                    m: candidate.m,
                    c: candidate.c,
                },
            );
        }
    }
}

/// The recorded candidates, best match first, ties by id so two runs that
/// see the same results write the same order.
fn top_candidates(seen: BTreeMap<String, CandidateRecord>) -> Vec<CandidateRecord> {
    let mut top: Vec<CandidateRecord> = seen.into_values().collect();
    top.sort_by(|left, right| {
        right
            .m
            .partial_cmp(&left.m)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.id.cmp(&right.id))
    });
    top.truncate(CANDIDATE_TOP);
    top
}

/// Confirm the first recorded candidate the content index vouches for.
/// The index decides on a hash, so no title gate applies; the evidence
/// record still keeps its best query and score.
fn apply_hash_index(index: &dyn ContentIndex, resolved: &mut [PboResolution]) {
    for item in resolved.iter_mut() {
        if item.resolution.status == ResolutionStatus::Confirmed {
            continue;
        }
        let vouched = item
            .candidates
            .iter()
            .find(|candidate| index.confirms(&item.evidence.fingerprint, &candidate.id))
            .map(|candidate| (candidate.id.clone(), candidate.title.clone(), candidate.m));
        let Some((id, title, m)) = vouched else {
            continue;
        };
        item.resolution = Resolution {
            status: ResolutionStatus::Confirmed,
            id: Some(id),
            title: Some(title),
            m,
            q: item.resolution.q,
            c: 1.0,
            margin: 0.0,
            query: item.resolution.query.clone(),
            matched_by: vec!["manifest_hash".to_string()],
        };
    }
}

/// The one candidate that two or more distinct queries agreed on, unless a
/// second candidate has the same claim. Two agreeing names are still a tie.
fn unique_agreement(seen: &BTreeMap<String, Agreement>) -> Option<(&str, &Agreement)> {
    // Agreement alone never confirms. Two queries agreeing on a generic
    // word is not provenance: five titles containing "markers" agree just
    // as readily. The agreed item must also carry strong corroboration
    // from the PBO, and a broad query cannot confirm what no single query
    // may confirm, so each agreement also needs a specific query and an
    // anchor the title actually matched.
    let mut agreed = seen.iter().filter(|(_, hit)| {
        hit.queries.len() >= CROSS_QUERY_MIN_QUERIES
            && hit.anchor_matched
            && hit.q >= MIN_Q
            && hit.corroborated
    });
    let (id, hit) = agreed.next()?;
    if agreed.next().is_some() {
        return None;
    }
    // Agreement must also stand clear of every rival the pool saw. Six
    // titles carry "Advanced Diving Environment" at a perfect score, and
    // two queries surfacing one of them says nothing about which one holds
    // this file. A rival inside the margin means the tie is still a tie.
    let lead = seen
        .iter()
        .filter(|(other, _)| other.as_str() != id.as_str())
        .map(|(_, rival)| hit.m - rival.m)
        .fold(f64::INFINITY, f64::min);
    if lead < MIN_MARGIN {
        return None;
    }
    Some((id.as_str(), hit))
}

/// Confirm a candidate that independent queries agreed on.
///
/// Agreement alone never confirms: the caller admits a hit only when
/// something in the PBO corroborates it (see [`unique_agreement`]). The
/// anchor gate is enforced by the `anchor_matched` requirement on the
/// agreement, not inside `decide`. The margin is reported but not
/// required, because agreement replaces separation here.
fn agreed(id: &str, hit: &Agreement, all: &BTreeMap<String, Agreement>) -> Resolution {
    let runner_up = all
        .iter()
        .filter(|(other, _)| other.as_str() != id)
        .map(|(_, other)| other.m)
        .fold(0.0_f64, f64::max);
    Resolution {
        status: ResolutionStatus::Confirmed,
        id: Some(id.to_string()),
        title: Some(hit.title.clone()),
        m: hit.m,
        q: hit.q,
        c: hit.m * hit.q,
        margin: (hit.m - runner_up).max(0.0),
        // The line can name only one query; the first in text order keeps
        // that choice independent of iteration order.
        query: hit.queries.first().cloned(),
        matched_by: vec!["cross_query".to_string()],
    }
}

/// Drop a shared title that no PBO evidence anchors.
///
/// "Gold Squadron" for `ffaa_data` and "Virtual Garage" for `rksla3-common`
/// are real Workshop titles that share no word with the file. A shared
/// title is only useful when it echoes the PBO's own class, prefix, stem or
/// CBA category.
fn prune_shared_title(resolution: &mut Resolution, evidence: &IdentityEvidence) {
    let Some(index) = resolution
        .matched_by
        .iter()
        .position(|signal| signal.starts_with("shared_title:"))
    else {
        return;
    };
    let title = resolution.matched_by[index]
        .trim_start_matches("shared_title:")
        .to_string();
    let anchors = evidence_anchors(evidence);
    let title_tokens: BTreeSet<String> = normalise(&title).tokens.into_iter().collect();
    if !anchors.iter().any(|anchor| title_tokens.contains(anchor)) {
        resolution.matched_by.remove(index);
    }
}

fn evidence_anchors(evidence: &IdentityEvidence) -> BTreeSet<String> {
    let mut anchors = BTreeSet::new();
    let mut add = |text: &str| {
        for token in normalise(text).tokens {
            if token.chars().count() >= 3 && !is_generic_word(&token) && !is_structural_stop(&token)
            {
                anchors.insert(token);
            }
        }
    };
    for phrase in &evidence.phrases {
        if matches!(
            phrase.source,
            PhraseSource::CbaOptionName
                | PhraseSource::CfgPatchesClass
                | PhraseSource::PrefixFamily
                | PhraseSource::Stem
        ) {
            add(&phrase.text);
        }
    }
    if let Some(prefix) = evidence.prefix.as_deref() {
        add(prefix);
    }
    if !evidence.stem.is_empty() {
        add(&evidence.stem);
    }
    anchors
}

/// One page of one query, from the memo or from the search endpoint.
fn memoised(ctx: &mut ResolveContext, query: &str) -> Result<BrowsePage, ResolutionStatus> {
    memoised_page(ctx, query, 1)
}

/// One page of one query, memoised by text and page number.
fn memoised_page(
    ctx: &mut ResolveContext,
    query: &str,
    page: u32,
) -> Result<BrowsePage, ResolutionStatus> {
    let key = page_key(query, page);
    if let Some(cached) = ctx.memo.get(&key) {
        return cached.outcome();
    }

    let mut outcome = if page == 1 {
        first_page(ctx, query)
    } else {
        fetch_outcome(ctx, query, page)
    };
    // Throttle guard: Steam sometimes answers a search with an empty page
    // while it throttles, and that looks like a real absence. A second
    // paced request tells the two apart. Only the first page is retried,
    // and only once. Keyed results already merged into this page keep it
    // from looking empty.
    if page == 1 && outcome.as_ref().is_ok_and(|held| held.total_count == 0) {
        ctx.limiter.pause(Duration::from_secs(30));
        outcome = fetch_outcome(ctx, query, page);
    }

    if let Ok(page) = &mut outcome {
        ctx.drop_excluded(&mut page.results);
    }
    ctx.memo.insert(
        key,
        match &outcome {
            Ok(page) => Memo::Page(page.clone()),
            Err(status) => Memo::Failed(*status),
        },
    );
    outcome
}

/// One request, with a throttle or transport failure mapped to a status.
fn fetch_outcome(
    ctx: &mut ResolveContext,
    query: &str,
    page: u32,
) -> Result<BrowsePage, ResolutionStatus> {
    match fetch_page(ctx, query, page) {
        Ok(page) => Ok(page),
        Err(error) => {
            // A silent `unresolved` hides a throttled endpoint from the
            // operator, so the reason goes to stderr.
            eprintln!("  query failed: \"{query}\" ({error:?})");
            match &error {
                SearchError::Http(http) if http.is_rate_limit() => {
                    Err(ResolutionStatus::RateLimited)
                }
                _ => Err(ResolutionStatus::Unresolved),
            }
        }
    }
}

/// Page one of one search: the browse page with the keyed results merged
/// in, or the keyed page alone when the browse page failed.
fn first_page(ctx: &mut ResolveContext, query: &str) -> Result<BrowsePage, ResolutionStatus> {
    let browse = fetch_outcome(ctx, query, 1);
    let keyed = ctx.keyed_page(query).unwrap_or_default();
    match browse {
        Ok(page) if keyed.is_empty() => Ok(page),
        Ok(page) => Ok(merge_page(page, &keyed)),
        Err(_status) if !keyed.is_empty() => Ok(keyed_only_page(keyed)),
        Err(status) => Err(status),
    }
}

/// Append the keyed results that the browse page did not already hold.
/// The browse copy of an item is kept, because it carries the description
/// and tags that the keyed response leaves out.
fn merge_page(mut page: BrowsePage, keyed: &[BrowseResult]) -> BrowsePage {
    let mut seen: HashSet<String> = page.results.iter().map(|item| item.id.clone()).collect();
    for item in keyed {
        if seen.insert(item.id.clone()) {
            page.results.push(item.clone());
        }
    }
    page.total_count = page.total_count.max(page.results.len() as u64);
    page
}

fn keyed_only_page(results: Vec<BrowseResult>) -> BrowsePage {
    BrowsePage {
        current_page: 1,
        total_pages: 1,
        total_count: results.len() as u64,
        results,
    }
}

/// The memo key for one page of one query. Page one keeps the bare text, so
/// two clusters that plan the same text still cost one request.
fn page_key(query: &str, page: u32) -> String {
    if page == 1 {
        query.to_string()
    } else {
        format!("{query}\u{1}{page}")
    }
}

/// One page of one search. The first page keeps the shorthand adapter.
fn fetch_page(ctx: &mut ResolveContext, query: &str, page: u32) -> Result<BrowsePage, SearchError> {
    if page == 1 {
        search(ctx.http, ctx.limiter, query)
    } else {
        search_browse_page(ctx.http, ctx.limiter, query, page)
    }
}

fn failed(status: ResolutionStatus, query: &str) -> Resolution {
    Resolution {
        status,
        id: None,
        title: None,
        m: 0.0,
        q: 0.0,
        c: 0.0,
        margin: 0.0,
        query: Some(query.to_string()),
        matched_by: Vec::new(),
    }
}

/// The Workshop id the PBO's own config points at, if any.
fn workshop_url_id(evidence: &IdentityEvidence) -> Option<String> {
    evidence
        .phrases
        .iter()
        .find_map(|phrase| phrase.workshop_id.clone())
}

/// Tokens present in at least 60% of a cluster of three or more PBOs.
fn group_terms(members: &[IdentityEvidence]) -> HashSet<String> {
    if members.len() < 3 {
        return HashSet::new();
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    for member in members {
        let mut seen: HashSet<String> = HashSet::new();
        for phrase in &member.phrases {
            for token in normalise(&phrase.text).tokens {
                if token.chars().count() >= 3 && seen.insert(token.clone()) {
                    *counts.entry(token).or_insert(0) += 1;
                }
            }
        }
    }

    let threshold = (members.len() * 60).div_ceil(100);
    counts
        .into_iter()
        .filter(|(_, count)| *count >= threshold)
        .map(|(token, _)| token)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigate::http::tests::FixtureHttp;
    use crate::investigate::http::HttpError;
    use crate::investigate::identity::{IdentityPhrase, PhraseSource};
    use crate::investigate::r#match::Corroboration;
    use crate::investigate::render::render_all;
    use crate::investigate::store::{merge, record_run, report_resolutions, IdentityStore};
    use std::time::Duration;

    const NOW: u64 = 1_700_000_000;
    const DAY: u64 = 24 * 3600;

    fn no_sleep(_: Duration) {}

    /// A site that answers by URL, so a test never depends on request
    /// order. Page one and page two of a search are separate pages, and an
    /// item page is addressed by id.
    struct Site {
        page_one: String,
        page_two: String,
        items: BTreeMap<String, String>,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl Site {
        fn new(page_one: String, page_two: String, items: Vec<(&str, String)>) -> Self {
            Self {
                page_one,
                page_two,
                items: items
                    .into_iter()
                    .map(|(id, html)| (id.to_string(), html))
                    .collect(),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }

        fn item_calls(&self) -> usize {
            self.calls
                .borrow()
                .iter()
                .filter(|url| url.contains("filedetails"))
                .count()
        }
    }

    impl Http for Site {
        fn get(&self, url: &str) -> Result<String, HttpError> {
            self.calls.borrow_mut().push(url.to_string());
            if url.contains("filedetails") {
                let id = url.rsplit("id=").next().unwrap_or_default();
                return self
                    .items
                    .get(id)
                    .cloned()
                    .ok_or_else(|| HttpError::Transport(format!("no item page for {id}")));
            }
            if url.contains("p=2") {
                return Ok(self.page_two.clone());
            }
            Ok(self.page_one.clone())
        }
    }

    fn page_html(results: &[(&str, &str)], total: u64) -> String {
        let typed: Vec<(&str, &str, u32)> =
            results.iter().map(|(id, title)| (*id, *title, 0)).collect();
        page_json(&typed, total)
    }

    /// A page whose results carry an explicit `file_type`.
    fn page_json(results: &[(&str, &str, u32)], total: u64) -> String {
        let items: Vec<serde_json::Value> = results
            .iter()
            .map(|(id, title, file_type)| {
                serde_json::json!({
                    "publishedfileid": id,
                    "title": title,
                    "file_type": file_type,
                    "subscriptions": 10,
                    "creator": "ADE Team",
                })
            })
            .collect();
        page_envelope(items, total)
    }

    /// A page whose results carry a short description.
    fn page_json_described(results: &[(&str, &str, &str)], total: u64) -> String {
        let items: Vec<serde_json::Value> = results
            .iter()
            .map(|(id, title, description)| {
                serde_json::json!({
                    "publishedfileid": id,
                    "title": title,
                    "file_type": 0,
                    "short_description": description,
                    "subscriptions": 10,
                    "creator": "ADE Team",
                })
            })
            .collect();
        page_envelope(items, total)
    }

    fn page_envelope(items: Vec<serde_json::Value>, total: u64) -> String {
        let inner = serde_json::json!({
            "queries": [{
                "queryKey": ["workshop_browse", {}, 1],
                "state": {"data": {
                    "current_page": 1,
                    "total_pages": 1,
                    "total_count": total,
                    "results": items,
                }},
            }],
        });
        let outer = serde_json::json!({"renderContext": {"queryData": inner.to_string()}});
        format!("<script type=\"application/json\" id=\"valve-ssr-data\">{outer}</script>")
    }

    fn evidence(pbo: &str, phrase: &str, source: PhraseSource) -> IdentityEvidence {
        IdentityEvidence {
            pbo: pbo.to_string(),
            fingerprint: format!("fp-{pbo}"),
            phrases: vec![IdentityPhrase {
                text: phrase.to_string(),
                source,
                workshop_id: None,
            }],
            cfgpatches_author: Some("ADE".to_string()),
            stem: "ade_core".to_string(),
            ..Default::default()
        }
    }

    fn confirmed_page() -> String {
        page_html(&[("1", "ADE - Advanced Diving Environment")], 200)
    }

    #[test]
    fn resolve_stops_after_first_confirmed_query() {
        let http = FixtureHttp::new(vec![Ok(confirmed_page()), Ok(confirmed_page())]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(http.calls().len(), 1);
    }

    #[test]
    fn the_investigated_item_is_never_a_candidate() {
        let page = page_html(&[("3549882948", "UKSF Mod Pack")], 1);
        let http = FixtureHttp::new(vec![Ok(page.clone()), Ok(page.clone()), Ok(page.clone())]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let item = || evidence("pdt_envg.pbo", "UKSF Mod Pack", PhraseSource::CfgModsName);

        // Control: without the exclusion the same page confirms, so the
        // assertion below measures the exclusion and not a weak fixture.
        let mut open = ResolveContext::new(&http, &mut limiter, &corpus, None);
        let resolved = resolve_all(vec![item()], &mut open);
        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);

        let http = FixtureHttp::new(vec![Ok(page.clone()), Ok(page.clone()), Ok(page)]);
        let mut limiter = RateLimiter::new(no_sleep);
        let mut closed = ResolveContext::new(&http, &mut limiter, &corpus, None);
        closed.exclude_item("3549882948".to_string());
        let resolved = resolve_all(vec![item()], &mut closed);

        assert_ne!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_ne!(resolved[0].resolution.id.as_deref(), Some("3549882948"));
    }

    #[test]
    fn refresh_drops_a_fresh_record() {
        let now = 1_700_000_000u64;
        let evidence = IdentityEvidence {
            pbo: "ade_core.pbo".to_string(),
            fingerprint: "fp".to_string(),
            stem: "ade_core".to_string(),
            ..Default::default()
        };
        let resolution = PboResolution {
            pbo: "ade_core.pbo".to_string(),
            fingerprint: "fp".to_string(),
            evidence: evidence.clone(),
            attempts: vec![QueryAttempt {
                text: "ADE diving".to_string(),
                kind: QueryKind::IdentityName,
                q: 0.9,
                total_count: Some(1),
                outcome: ResolutionStatus::Confirmed,
            }],
            resolution: Resolution {
                status: ResolutionStatus::Confirmed,
                id: Some("1891978580".to_string()),
                title: Some("ADE - Advanced Diving Environment".to_string()),
                m: 1.0,
                q: 0.9,
                c: 0.9,
                margin: 1.0,
                query: Some("ADE diving".to_string()),
                matched_by: vec!["title".to_string()],
            },
            candidates: Vec::new(),
        };
        let mut store = IdentityStore::empty();
        store.pbo.insert(
            "ade_core.pbo".to_string(),
            crate::investigate::store::PboEntry::from_resolution(&resolution, now),
        );

        let (kept, pending) = split_fresh_with(&store, vec![evidence.clone()], now, false);
        assert_eq!(kept.len(), 1, "a matching fingerprint keeps its record");
        assert!(pending.is_empty());

        let (kept, pending) = split_fresh_with(&store, vec![evidence], now, true);
        assert!(kept.is_empty(), "--refresh must drop the stored record");
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn resolve_uses_at_most_three_queries() {
        let page = page_html(&[("9", "Something Unrelated")], 50);
        let http = FixtureHttp::new(vec![Ok(page.clone()), Ok(page.clone()), Ok(page)]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        let mut item = evidence("bettir.pbo", "BettIR Core", PhraseSource::CfgModsName);
        item.phrases.push(IdentityPhrase {
            text: "BETTIR".to_string(),
            source: PhraseSource::TagAuthor,
            workshop_id: None,
        });
        item.phrases.push(IdentityPhrase {
            text: "bettir_core".to_string(),
            source: PhraseSource::CfgPatchesClass,
            workshop_id: None,
        });

        let resolved = resolve_all(vec![item], &mut ctx);

        assert!(resolved[0].attempts.len() <= 3);
        assert!(http.calls().len() <= 3);
        assert_ne!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
    }

    #[test]
    fn resolve_marks_rate_limited() {
        // One canned response per attempt: the first call plus five retries.
        let http = FixtureHttp::new(vec![
            Err(HttpError::Status(429)),
            Err(HttpError::Status(429)),
            Err(HttpError::Status(429)),
            Err(HttpError::Status(429)),
            Err(HttpError::Status(429)),
            Err(HttpError::Status(429)),
        ]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::RateLimited);
        assert!(resolved[0].resolution.title.is_none());
    }

    #[test]
    fn resolve_dedupes_identical_queries() {
        let http = FixtureHttp::new(vec![Ok(confirmed_page())]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(
            vec![
                evidence("ade_a.pbo", "ADE diving", PhraseSource::CfgModsName),
                evidence("ade_b.pbo", "ADE diving", PhraseSource::CfgModsName),
            ],
            &mut ctx,
        );

        assert_eq!(resolved.len(), 2);
        assert_eq!(http.calls().len(), 1);
    }

    #[test]
    fn resolve_reports_attempt_status_when_nothing_scores() {
        let page = page_html(&[("9", "Unrelated Title")], 50);
        let http = FixtureHttp::new(vec![Ok(page)]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        assert_eq!(resolved[0].attempts.len(), 1);
        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Ambiguous);
        assert!(resolved[0].resolution.title.is_none());
    }

    #[test]
    fn zero_result_page_is_retried_once() {
        // Steam answers the first search with an empty page while it
        // throttles. The retry sees the real result, so the PBO resolves.
        let http = FixtureHttp::new(vec![Ok(page_html(&[], 0)), Ok(confirmed_page())]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        assert_eq!(http.calls().len(), 2);
        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
    }

    #[test]
    fn zero_result_retry_happens_once() {
        // Both attempts come back empty. The guard must not retry a third
        // time, and the empty page is treated as a real absence.
        let http = FixtureHttp::new(vec![Ok(page_html(&[], 0)), Ok(page_html(&[], 0))]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        // Page one is fetched twice; the deeper page is a separate request.
        let page_one = http
            .calls()
            .iter()
            .filter(|url| !url.contains("p=2"))
            .count();
        assert_eq!(page_one, 2);
        assert_eq!(resolved[0].resolution.status, ResolutionStatus::NoResult);
    }

    struct PanicHttp;
    impl Http for PanicHttp {
        fn get(&self, _url: &str) -> Result<String, HttpError> {
            panic!("a PBO with no query must not make a request");
        }
    }

    #[test]
    fn resolve_no_query_never_calls_http() {
        let http = PanicHttp;
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        let mut item = IdentityEvidence {
            pbo: "opaque.pbo".to_string(),
            fingerprint: "fp".to_string(),
            ..Default::default()
        };
        item.stem.clear();

        let resolved = resolve_all(vec![item], &mut ctx);

        assert_eq!(
            resolved[0].resolution.status,
            ResolutionStatus::NoDistinctiveQuery
        );
        assert!(resolved[0].attempts.is_empty());
        assert!(resolved[0].candidates.is_empty());
    }

    #[test]
    fn freshness_skips_confirmed_pbo() {
        let store = store_with(&[stored(
            "ade_core.pbo",
            ResolutionStatus::Confirmed,
            Some("ADE - Advanced Diving Environment"),
        )]);
        let evidence = vec![evidence(
            "ade_core.pbo",
            "ADE diving",
            PhraseSource::CfgModsName,
        )];

        let (kept, pending) = split_fresh(&store, evidence, NOW + 30 * DAY);

        assert!(pending.is_empty());
        assert_eq!(kept.len(), 1);
        assert_eq!(
            render_all(&kept, false),
            "[confirmed] ade_core.pbo -> \"ADE - Advanced Diving Environment\" (1891978580) [query \"ADE diving\", M=1.00 Q=0.83 margin=1.00]"
        );

        // A kept record never reaches the resolver, so no request is spent.
        let http = NeverHttp;
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        ctx.schedule_from(&store);
        assert!(resolve_all(pending, &mut ctx).is_empty());
    }

    #[test]
    fn freshness_retries_rate_limited_next_run() {
        let store = store_with(&[stored("ade_core.pbo", ResolutionStatus::RateLimited, None)]);

        let (kept, pending) = split_fresh(
            &store,
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            NOW + 60,
        );

        assert!(kept.is_empty());
        let http = FixtureHttp::new(vec![Ok(confirmed_page())]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(pending, &mut ctx);

        assert_eq!(http.calls().len(), 1);
        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
    }

    #[test]
    fn freshness_retries_unresolved_after_seven_days() {
        let store = store_with(&[stored("opaque.pbo", ResolutionStatus::Unresolved, None)]);
        let item = || evidence("opaque.pbo", "ADE diving", PhraseSource::CfgModsName);

        let (kept, pending) = split_fresh(&store, vec![item()], NOW + 6 * DAY);
        assert_eq!(kept.len(), 1);
        assert!(pending.is_empty());

        let (kept, pending) = split_fresh(&store, vec![item()], NOW + 8 * DAY);
        assert!(kept.is_empty());
        assert_eq!(pending.len(), 1);

        let http = FixtureHttp::new(vec![Ok(confirmed_page())]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        resolve_all(pending, &mut ctx);
        assert_eq!(http.calls().len(), 1);
    }

    #[test]
    fn cross_query_does_not_break_a_family_tie() {
        // Six ADE variants share the exact title, and two queries surface
        // several of them. One 203-PBO run confirmed a re-upload from this
        // shape while the human label named the original, so agreement must
        // leave the line ambiguous instead of picking a family member.
        let item = multi_query_evidence();
        let texts = planned_texts(&item);
        assert!(texts.len() >= CROSS_QUERY_MIN_QUERIES);

        let responses = texts
            .iter()
            .enumerate()
            .map(|(index, text)| Ok(page_html(&[("1", text), (DECOYS[index], text)], 200)))
            .collect();
        let http = FixtureHttp::new(responses);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![item], &mut ctx);

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Ambiguous);
        assert!(resolved[0].resolution.id.is_none());
        assert!(!resolved[0]
            .resolution
            .matched_by
            .iter()
            .any(|signal| signal == "cross_query"));
    }

    #[test]
    fn cross_query_rejects_two_agreeing_ids() {
        let item = multi_query_evidence();
        let texts = planned_texts(&item);
        assert!(texts.len() >= CROSS_QUERY_MIN_QUERIES);

        let responses = texts
            .iter()
            .map(|text| Ok(page_html(&[("1", text), ("2", text)], 200)))
            .collect();
        let http = FixtureHttp::new(responses);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![item], &mut ctx);

        assert_ne!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert!(resolved[0].resolution.title.is_none());
        assert!(resolved[0].resolution.id.is_none());
        assert!(!resolved[0]
            .resolution
            .matched_by
            .iter()
            .any(|signal| signal == "cross_query"));
    }

    #[test]
    fn cross_query_never_confirms_non_mod() {
        let item = multi_query_evidence();
        let texts = planned_texts(&item);
        assert!(texts.len() >= CROSS_QUERY_MIN_QUERIES);

        // The same id leads every query with a perfect title match, but it
        // is not a mod, so neither path may name it.
        let responses = texts
            .iter()
            .map(|text| Ok(page_json(&[("1", text, 1)], 200)))
            .collect();
        let http = FixtureHttp::new(responses);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![item], &mut ctx);

        assert_ne!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert!(resolved[0].resolution.title.is_none());
    }

    #[test]
    fn candidates_are_recorded_and_reported() {
        let page = page_html(
            &[
                ("1", "ADE - Advanced Diving Environment"),
                ("7", "Some Other Diving Tool"),
            ],
            200,
        );
        let http = FixtureHttp::new(vec![Ok(page)]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(resolved[0].candidates.len(), 2);
        assert_eq!(resolved[0].candidates[0].id, "1");
        assert_eq!(resolved[0].candidates[0].m, 1.0);
        assert_eq!(resolved[0].candidates[1].id, "7");

        // The run writes the list, and a report reads it back in the same
        // order. A score may land one unit in the last place away through
        // JSON, so the ranks are compared, not the float bits.
        let mut store = IdentityStore::empty();
        let rendered = record_run(&mut store, &resolved, &[], NOW);
        let ids = |list: &[CandidateRecord]| -> Vec<String> {
            list.iter().map(|candidate| candidate.id.clone()).collect()
        };
        assert_eq!(ids(&rendered[0].candidates), ids(&resolved[0].candidates));

        let reported = report_resolutions(&store, &examined(&["ade_core.pbo"]));
        assert_eq!(ids(&reported[0].candidates), ["1", "7"]);
        assert_eq!(render_all(&reported, false), render_all(&rendered, false));
    }

    #[test]
    fn candidates_are_deduped_and_sorted() {
        let mut seen: BTreeMap<String, CandidateRecord> = BTreeMap::new();
        record_candidates(
            &mut seen,
            &[ranked("2", 0.90), ranked("1", 0.90), ranked("3", 0.50)],
        );
        // A later query ranks "2" lower: the best score is the one kept.
        record_candidates(
            &mut seen,
            &[
                ranked("2", 0.70),
                ranked("4", 0.85),
                ranked("5", 0.10),
                ranked("6", 0.05),
                ranked("7", 0.01),
            ],
        );

        let top = top_candidates(seen);

        let ids: Vec<&str> = top.iter().map(|candidate| candidate.id.as_str()).collect();
        assert_eq!(ids, ["1", "2", "4", "3", "5", "6", "7"]);
        assert_eq!(top[1].m, 0.90);
    }

    /// A ranked candidate with only the fields the recorder reads.
    fn ranked(id: &str, m: f64) -> RankedCandidate {
        RankedCandidate {
            id: id.to_string(),
            title: format!("Title {id}"),
            file_type: 0,
            m,
            c: m,
            head: 1.0,
            anchor_matched: true,
            corroboration: Corroboration::default(),
        }
    }

    /// Every PBO with its stored fingerprint, for a report of the whole store.
    fn examined(pbos: &[&str]) -> Vec<(String, String)> {
        pbos.iter()
            .map(|pbo| ((*pbo).to_string(), format!("fp-{pbo}")))
            .collect()
    }

    /// Decoy ids for the agreement fixtures. One per planned query, so no
    /// decoy is ranked by two queries.
    const DECOYS: [&str; 3] = ["91", "92", "93"];

    /// Evidence whose plan holds more than one distinct query. The required
    /// addon supplies the corroboration an agreement needs, so a confirmed
    /// line here comes from agreement rather than a strong title alone.
    fn multi_query_evidence() -> IdentityEvidence {
        let mut item = evidence("bettir.pbo", "BettIR Core", PhraseSource::CfgModsName);
        item.required_addons = vec!["BettIR".to_string()];
        item.phrases.push(IdentityPhrase {
            text: "BIR".to_string(),
            source: PhraseSource::TagAuthor,
            workshop_id: None,
        });
        item.phrases.push(IdentityPhrase {
            text: "Diving Mask".to_string(),
            source: PhraseSource::DisplayName,
            workshop_id: None,
        });
        item.phrases.push(IdentityPhrase {
            text: "BettIR_Devices".to_string(),
            source: PhraseSource::CfgPatchesClass,
            workshop_id: None,
        });
        item.stem = "bettir_core".to_string();
        item
    }

    /// The query texts the resolver will send for this evidence.
    fn planned_texts(item: &IdentityEvidence) -> Vec<String> {
        let terms = group_terms(std::slice::from_ref(item));
        plan_queries(
            item,
            &PlannerContext {
                corpus: &CorpusStats::default(),
                group_terms: &terms,
                pool: None,
            },
        )
        .queries
        .iter()
        .map(|query| query.text.clone())
        .collect()
    }

    /// A stored record as one run would write it.
    fn store_with(resolutions: &[PboResolution]) -> IdentityStore {
        let mut store = IdentityStore::empty();
        merge(&mut store, resolutions, &[], NOW);
        store
    }

    fn stored(pbo: &str, status: ResolutionStatus, title: Option<&str>) -> PboResolution {
        let evidence = evidence(pbo, "ADE diving", PhraseSource::CfgModsName);
        PboResolution {
            pbo: pbo.to_string(),
            fingerprint: evidence.fingerprint.clone(),
            evidence,
            attempts: vec![QueryAttempt {
                text: "ADE diving".to_string(),
                kind: QueryKind::IdentityName,
                q: 0.83,
                total_count: Some(200),
                outcome: status,
            }],
            resolution: Resolution {
                status,
                id: title.map(|_| "1891978580".to_string()),
                title: title.map(str::to_string),
                m: 1.0,
                q: 0.83,
                c: 0.83,
                margin: 1.0,
                query: Some("ADE diving".to_string()),
                matched_by: vec!["title".to_string()],
            },
            candidates: Vec::new(),
        }
    }

    struct NeverHttp;
    impl Http for NeverHttp {
        fn get(&self, _url: &str) -> Result<String, HttpError> {
            panic!("a fresh record must not make a request");
        }
    }

    /// The creator panel of an item page, cut down to what the parser reads.
    fn item_page(creator: &str) -> String {
        format!(
            r#"<div class="rightDetailsBlock"><div class="creatorsBlock">
                <div class="friendBlock persona offline" data-miniprofile="57858904">
                    <div class="friendBlockContent">
                        {creator}<br>
                        <span class="friendSmallText">Offline</span>
                    </div>
                </div>
            </div></div>"#
        )
    }

    /// A PBO whose config author is `MadMax` and whose single phrase scores
    /// the titled candidate at a perfect match.
    fn authored_evidence() -> IdentityEvidence {
        let mut item = evidence("ade_core.pbo", "ADE diving", PhraseSource::CfgModsName);
        item.cfgpatches_author = Some("MadMax".to_string());
        item
    }

    #[test]
    fn author_match_promotes_to_confirmed() {
        let http = FixtureHttp::new(vec![
            Ok(page_html(
                &[("1", "ADE - Advanced Diving Environment")],
                200,
            )),
            Ok(item_page("MadMax")),
        ]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![authored_evidence()], &mut ctx);

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(resolved[0].resolution.id.as_deref(), Some("1"));
        assert!(resolved[0]
            .resolution
            .matched_by
            .iter()
            .any(|signal| signal == "author"));
        assert_eq!(http.calls().len(), 2);
        assert!(http.calls()[1].contains("sharedfiles/filedetails/?id=1"));
    }

    /// An item page with a creator name and a description.
    fn item_page_with(creator: &str, description: &str) -> String {
        format!(
            r#"{panel}<div class="workshopItemDescription" id="highlightContent">{description}</div>"#,
            panel = item_page(creator)
        )
    }

    /// A PBO whose CfgPatches class carries the content word `diving`.
    fn content_evidence() -> IdentityEvidence {
        let mut item = evidence("ade_core.pbo", "ADE diving", PhraseSource::CfgModsName);
        item.cfgpatches_author = Some("MadMax".to_string());
        item.prefix = Some("ADE".to_string());
        item.phrases.push(IdentityPhrase {
            text: "ade_diving_trench".to_string(),
            source: PhraseSource::CfgPatchesClass,
            workshop_id: None,
        });
        item
    }

    /// Two results that hold the same title and the same score.
    fn tied_page() -> String {
        page_html(
            &[
                ("1", "ADE - Advanced Diving Trench Environment"),
                ("2", "ADE - Advanced Diving Environment"),
            ],
            40,
        )
    }

    #[test]
    fn tie_description_confirms_unique_candidate() {
        let http = Site::new(
            tied_page(),
            page_html(&[], 0),
            vec![
                (
                    "1",
                    item_page_with("Someone Else", "Diving equipment for submarines."),
                ),
                (
                    "2",
                    item_page_with("Another Person", "A terrain collection."),
                ),
            ],
        );
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![content_evidence()], &mut ctx);

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(resolved[0].resolution.id.as_deref(), Some("1"));
        assert!(resolved[0]
            .resolution
            .matched_by
            .iter()
            .any(|signal| signal == "item_description"));
        // Both tied pages are read: one match confirms only because the
        // other candidate did not match.
        assert_eq!(http.item_calls(), 2);
    }

    #[test]
    fn tie_author_confirms_unique_candidate() {
        let http = Site::new(
            tied_page(),
            page_html(&[], 0),
            vec![
                (
                    "1",
                    item_page_with("MadMax", "An addon for advanced players."),
                ),
                (
                    "2",
                    item_page_with("Another Person", "A terrain collection."),
                ),
            ],
        );
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![content_evidence()], &mut ctx);

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(resolved[0].resolution.id.as_deref(), Some("1"));
        assert!(resolved[0]
            .resolution
            .matched_by
            .iter()
            .any(|signal| signal == "item_author"));
    }

    #[test]
    fn tie_without_evidence_stays_ambiguous() {
        let http = Site::new(
            tied_page(),
            page_html(&[], 0),
            vec![
                (
                    "1",
                    item_page_with("Someone Else", "An addon for advanced players."),
                ),
                (
                    "2",
                    item_page_with("Another Person", "A terrain collection."),
                ),
            ],
        );
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![content_evidence()], &mut ctx);

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Ambiguous);
        assert!(resolved[0].resolution.id.is_none());
        assert!(resolved[0]
            .resolution
            .matched_by
            .iter()
            .any(|signal| signal.starts_with("shared_title:")));
    }

    #[test]
    fn author_lookup_allowed_on_tie() {
        let q = score_query("ADE diving", Some(40), &CorpusStats::default());
        let tied = [ranked("1", 1.0), ranked("2", 1.0)];

        // A tie is a small margin: the page may be read, because content is
        // the only evidence that can separate the candidates.
        assert!(awaiting_corroboration(&tied, &q).is_some());

        // A candidate that fails a title gate is never offered.
        let weak = [ranked("1", 0.70)];
        assert!(awaiting_corroboration(&weak, &q).is_none());
    }

    #[test]
    fn candidates_keep_top_ten() {
        let seen: Vec<RankedCandidate> = (1..=12)
            .map(|rank| ranked(&format!("{rank:02}"), 1.0 - f64::from(rank) / 100.0))
            .collect();
        let mut recorded: BTreeMap<String, CandidateRecord> = BTreeMap::new();

        record_candidates(&mut recorded, &seen);
        let top = top_candidates(recorded);

        assert_eq!(top.len(), 10);
        assert_eq!(top[9].id, "10");
    }

    /// A PBO with no config author, so no item page is read for it.
    fn anonymous_evidence() -> IdentityEvidence {
        let mut item = evidence("ade_core.pbo", "ADE diving", PhraseSource::CfgModsName);
        item.cfgpatches_author = None;
        item
    }

    #[test]
    fn page_two_finds_missing_candidate() {
        let http = Site::new(
            page_json_described(&[("9", "Unrelated Title", "")], 4),
            page_json_described(
                &[(
                    "1",
                    "ADE - Advanced Diving Environment",
                    "Requires ade_core.",
                )],
                4,
            ),
            Vec::new(),
        );
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![anonymous_evidence()], &mut ctx);

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(resolved[0].resolution.id.as_deref(), Some("1"));
        assert_eq!(resolved[0].attempts.len(), 2);
        // The extra attempt counts the merged set, not page two alone.
        assert_eq!(resolved[0].attempts[1].total_count, Some(2));
        assert_eq!(http.calls().len(), 2);
        assert!(http.calls()[1].contains("p=2"));
    }

    #[test]
    fn page_two_prints_no_unanchored_shared_title() {
        let http = Site::new(
            page_json(&[("9", "Unrelated Title", 1)], 4),
            page_json(
                &[
                    ("1", "Diving Gear Systems", 0),
                    ("2", "Diving Gear Systems", 0),
                ],
                4,
            ),
            Vec::new(),
        );
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![anonymous_evidence()], &mut ctx);

        assert_eq!(resolved[0].attempts.len(), 2);
        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Ambiguous);
        assert!(!resolved[0]
            .resolution
            .matched_by
            .iter()
            .any(|signal| signal.starts_with("shared_title:")));
    }

    #[test]
    fn cross_query_rejects_weak_tie_without_corroboration() {
        let mut seen: BTreeMap<String, Agreement> = BTreeMap::new();
        seen.insert(
            "1".to_string(),
            Agreement {
                title: "Alpha Charlie".to_string(),
                m: 0.47,
                q: 0.8,
                queries: ["a", "b"].into_iter().map(str::to_string).collect(),
                anchor_matched: true,
                corroborated: false,
            },
        );

        assert!(unique_agreement(&seen).is_none());

        seen.get_mut("1").unwrap().corroborated = true;
        assert!(unique_agreement(&seen).is_some());
    }

    #[test]
    fn cross_query_rejects_agreement_from_a_broad_query() {
        let mut seen: BTreeMap<String, Agreement> = BTreeMap::new();
        seen.insert(
            "1".to_string(),
            Agreement {
                title: "Alpha Charlie".to_string(),
                m: 1.0,
                q: MIN_Q - 0.01,
                queries: ["a", "b"].into_iter().map(str::to_string).collect(),
                anchor_matched: true,
                corroborated: true,
            },
        );

        assert!(unique_agreement(&seen).is_none());

        seen.get_mut("1").unwrap().q = 0.8;
        assert!(unique_agreement(&seen).is_some());
    }

    #[test]
    fn cross_query_rejects_agreement_without_an_anchor() {
        let mut seen: BTreeMap<String, Agreement> = BTreeMap::new();
        seen.insert(
            "1".to_string(),
            Agreement {
                title: "Alpha Charlie".to_string(),
                m: 1.0,
                q: 0.8,
                queries: ["a", "b"].into_iter().map(str::to_string).collect(),
                anchor_matched: false,
                corroborated: true,
            },
        );

        assert!(unique_agreement(&seen).is_none());

        seen.get_mut("1").unwrap().anchor_matched = true;
        assert!(unique_agreement(&seen).is_some());
    }

    #[test]
    fn cross_query_rejects_agreement_tied_with_a_rival_in_the_pool() {
        // Six ADE variants share the exact title at a perfect score. Two
        // queries agreeing on one of them is still a six-way tie.
        let mut seen: BTreeMap<String, Agreement> = BTreeMap::new();
        for id in ["1", "2", "3"] {
            seen.insert(
                id.to_string(),
                Agreement {
                    title: "Advanced Diving Environment".to_string(),
                    m: 1.0,
                    q: 0.9,
                    queries: ["ade diving".to_string(), "diving mask".to_string()]
                        .into_iter()
                        .collect(),
                    anchor_matched: true,
                    corroborated: true,
                },
            );
        }
        assert!(unique_agreement(&seen).is_none());
    }

    #[test]
    fn cross_query_accepts_agreement_that_leads_the_pool() {
        let mut seen: BTreeMap<String, Agreement> = BTreeMap::new();
        seen.insert(
            "1".to_string(),
            Agreement {
                title: "DEVGRU Red Squadron (121)".to_string(),
                m: 1.0,
                q: 0.9,
                queries: ["devgru".to_string(), "devgru redsquadron".to_string()]
                    .into_iter()
                    .collect(),
                anchor_matched: true,
                corroborated: true,
            },
        );
        seen.insert(
            "2".to_string(),
            Agreement {
                title: "@DEVGRU CQB Objects".to_string(),
                m: 0.57,
                q: 0.7,
                queries: ["devgru".to_string()].into_iter().collect(),
                anchor_matched: true,
                corroborated: false,
            },
        );

        assert_eq!(unique_agreement(&seen).map(|(id, _)| id), Some("1"));
    }

    #[test]
    fn cross_query_rejects_agreement_on_a_generic_title_without_corroboration() {
        // Five titles carry "backpacks" at a perfect score, and two queries
        // agree on one of them. Nothing in the PBO names any of them, so the
        // agreement adds nothing and the line must stay ambiguous.
        let mut seen: BTreeMap<String, Agreement> = BTreeMap::new();
        for id in ["1", "2", "3", "4", "5"] {
            seen.insert(
                id.to_string(),
                Agreement {
                    title: format!("Backpacks {id}"),
                    m: 1.0,
                    q: 0.8,
                    queries: ["backpacks".to_string()].into_iter().collect(),
                    anchor_matched: true,
                    corroborated: false,
                },
            );
        }
        seen.get_mut("1")
            .unwrap()
            .queries
            .insert("uksf backpacks".to_string());

        assert!(unique_agreement(&seen).is_none());
    }

    #[test]
    fn agreement_ignores_a_shared_stem_or_a_shared_rare_token() {
        let shared_stem = crate::investigate::r#match::Corroboration {
            stem_in_description: true,
            ..Default::default()
        };
        assert!(!provenance_link(&shared_stem));

        let shared_words = crate::investigate::r#match::Corroboration {
            evidence_overlap: crate::investigate::r#match::EVIDENCE_MIN_TOKENS,
            evidence_tokens: vec!["markers".to_string()],
            ..Default::default()
        };
        assert!(!provenance_link(&shared_words));
    }

    #[test]
    fn agreement_accepts_a_dependency_or_uploader_link() {
        let dependency = crate::investigate::r#match::Corroboration {
            dep_match: true,
            ..Default::default()
        };
        assert!(provenance_link(&dependency));

        let uploader = crate::investigate::r#match::Corroboration {
            author_match: true,
            ..Default::default()
        };
        assert!(provenance_link(&uploader));
    }

    #[test]
    fn tie_generic_token_does_not_discriminate() {
        let mut item = evidence("ade_core.pbo", "ADE diving", PhraseSource::CfgModsName);
        item.cfgpatches_author = Some("MadMax".to_string());
        item.prefix = Some("ADE".to_string());
        item.phrases.push(IdentityPhrase {
            text: "ade_diving_gear".to_string(),
            source: PhraseSource::CfgPatchesClass,
            workshop_id: None,
        });
        let page = page_html(
            &[
                ("1", "ADE - Advanced Diving Environment"),
                ("2", "ADE - Advanced Diving Environment"),
            ],
            40,
        );
        let http = Site::new(
            page,
            page_html(&[], 0),
            vec![
                (
                    "1",
                    item_page_with("MadMax", "Diving equipment for submarines."),
                ),
                ("2", item_page_with("Another Person", "Diving tools.")),
            ],
        );
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(vec![item], &mut ctx);

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Ambiguous);
        assert!(resolved[0]
            .resolution
            .matched_by
            .iter()
            .any(|signal| signal.starts_with("shared_title:")));
    }

    #[test]
    fn resolve_reports_progress_per_cluster() {
        let http = FixtureHttp::new(Vec::new());
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let seen: std::cell::RefCell<Vec<(usize, usize, String)>> =
            std::cell::RefCell::new(Vec::new());
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        ctx.set_progress(Box::new(|progress: &Progress| {
            seen.borrow_mut()
                .push((progress.done, progress.total, progress.label.clone()));
        }));

        let resolved = resolve_all(
            vec![
                evidence("alpha_core.pbo", "Alpha Diving", PhraseSource::CfgModsName),
                evidence("bravo_core.pbo", "Bravo Tanking", PhraseSource::CfgModsName),
                evidence(
                    "charlie_core.pbo",
                    "Charlie Radios",
                    PhraseSource::CfgModsName,
                ),
            ],
            &mut ctx,
        );
        drop(ctx);

        let seen = seen.into_inner();
        assert_eq!(resolved.len(), 3);
        assert_eq!(seen.first().map(|step| (step.0, step.1)), Some((0, 3)));
        assert_eq!(seen.first().map(|step| step.2.as_str()), Some(""));
        assert!(seen.windows(2).all(|pair| pair[1].0 > pair[0].0));
        assert_eq!(seen.last().map(|step| (step.0, step.1)), Some((3, 3)));
        let labels: Vec<&str> = seen.iter().skip(1).map(|step| step.2.as_str()).collect();
        assert_eq!(labels, ["alpha_core", "bravo_core", "charlie_core"]);
    }

    #[test]
    fn resolve_without_progress_sink_stays_silent() {
        let http = FixtureHttp::new(Vec::new());
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);

        let resolved = resolve_all(
            vec![evidence(
                "alpha_core.pbo",
                "Alpha Diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        assert_eq!(resolved.len(), 1);
    }
    /// A keyed source that answers from a table, so a test never opens a
    /// socket and records every query it was sent.
    struct KeyedStub {
        pages: BTreeMap<String, Result<BrowsePage, KeyedError>>,
        calls: std::cell::RefCell<Vec<String>>,
    }

    impl KeyedStub {
        fn new(pages: Vec<(&str, Result<BrowsePage, KeyedError>)>) -> Self {
            Self {
                pages: pages
                    .into_iter()
                    .map(|(text, page)| (text.to_string(), page))
                    .collect(),
                calls: std::cell::RefCell::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl KeyedSearch for KeyedStub {
        fn query(&self, text: &str) -> Result<BrowsePage, KeyedError> {
            self.calls.borrow_mut().push(text.to_string());
            self.pages
                .get(text)
                .cloned()
                .unwrap_or_else(|| Ok(BrowsePage::default()))
        }
    }

    fn keyed_page(ids: &[(&str, &str)]) -> BrowsePage {
        BrowsePage {
            current_page: 1,
            total_pages: 1,
            total_count: ids.len() as u64,
            results: ids
                .iter()
                .map(|(id, title)| BrowseResult {
                    id: id.to_string(),
                    title: title.to_string(),
                    short_description: String::new(),
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
                    creator: "ADE Team".to_string(),
                    file_type: 0,
                })
                .collect(),
        }
    }

    fn resolve_with_keyed(keyed: &KeyedStub) -> Vec<PboResolution> {
        let http = FixtureHttp::new(vec![Ok(page_html(&[], 0))]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        ctx.use_keyed_search(keyed);
        resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        )
    }

    #[test]
    fn keyed_search_supplies_the_confirming_candidate() {
        let keyed = KeyedStub::new(vec![(
            "ADE diving",
            Ok(keyed_page(&[("1", "ADE - Advanced Diving Environment")])),
        )]);

        let resolved = resolve_with_keyed(&keyed);

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(resolved[0].resolution.id.as_deref(), Some("1"));
        assert_eq!(keyed.calls(), ["ADE diving"]);
    }

    #[test]
    fn a_keyed_top_hit_is_not_promoted_without_evidence() {
        let keyed = KeyedStub::new(vec![(
            "ADE diving",
            Ok(keyed_page(&[("9", "Unrelated Terrain Pack")])),
        )]);

        let resolved = resolve_with_keyed(&keyed);

        assert_ne!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(resolved[0].resolution.id, None);
    }

    #[test]
    fn keyed_report_states_an_empty_search() {
        let http = FixtureHttp::new(vec![Ok(page_html(&[], 0))]);
        let keyed = KeyedStub::new(vec![("ADE diving", Ok(BrowsePage::default()))]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        ctx.use_keyed_search(&keyed);

        resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );
        let report = ctx.keyed_report();

        assert_eq!(report.len(), 1);
        assert_eq!(report[0].queries, ["ADE diving"]);
        assert!(report[0].ranked.is_empty());
    }

    #[test]
    fn keyed_report_lists_the_queries_and_the_ranking() {
        let http = FixtureHttp::new(vec![Ok(page_html(&[], 0))]);
        let keyed = KeyedStub::new(vec![(
            "ADE diving",
            Ok(keyed_page(&[("1", "ADE - Advanced Diving Environment")])),
        )]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        ctx.use_keyed_search(&keyed);

        resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );
        let report = ctx.keyed_report();

        assert_eq!(report.len(), 1);
        assert_eq!(report[0].pbo, "ade_core.pbo");
        assert_eq!(report[0].queries, ["ADE diving"]);
        assert_eq!(
            report[0].ranked[0],
            (
                "1".to_string(),
                "ADE - Advanced Diving Environment".to_string()
            )
        );
    }

    #[test]
    fn a_refused_key_is_fatal_and_stops_further_keyed_requests() {
        let http = FixtureHttp::new(vec![Ok(page_html(&[], 0))]);
        let keyed = KeyedStub::new(vec![("ADE diving", Err(KeyedError::Rejected))]);
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        ctx.use_keyed_search(&keyed);

        resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        assert!(ctx.keyed_failure().is_some());
        assert_eq!(keyed.calls(), ["ADE diving"]);
    }

    #[test]
    fn a_keyed_result_already_on_the_browse_page_is_not_repeated() {
        let page = BrowsePage {
            current_page: 1,
            total_pages: 1,
            total_count: 1,
            results: keyed_page(&[("1", "ADE - Advanced Diving Environment")]).results,
        };

        let merged = merge_page(
            page,
            &keyed_page(&[("1", "ADE - Advanced Diving Environment")]).results,
        );

        assert_eq!(merged.results.len(), 1);
    }

    struct HashIndex {
        ids: Vec<&'static str>,
    }

    impl ContentIndex for HashIndex {
        fn confirms(&self, _fingerprint: &str, item: &str) -> bool {
            self.ids.contains(&item)
        }
    }

    #[test]
    fn a_content_hash_confirms_a_candidate_the_evidence_does_not() {
        let http = FixtureHttp::new(vec![Ok(page_html(&[], 0))]);
        let keyed = KeyedStub::new(vec![(
            "ADE diving",
            Ok(keyed_page(&[
                ("9", "Unrelated Terrain Pack"),
                ("2", "Repacked Addon Bundle"),
            ])),
        )]);
        let index = HashIndex { ids: vec!["2"] };
        let mut limiter = RateLimiter::new(no_sleep);
        let corpus = CorpusStats::default();
        let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, None);
        ctx.use_keyed_search(&keyed);
        ctx.content_index = Some(&index);

        let resolved = resolve_all(
            vec![evidence(
                "ade_core.pbo",
                "ADE diving",
                PhraseSource::CfgModsName,
            )],
            &mut ctx,
        );

        assert_eq!(resolved[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(resolved[0].resolution.id.as_deref(), Some("2"));
        assert_eq!(resolved[0].resolution.matched_by, ["manifest_hash"]);
    }
}

#[cfg(test)]
mod shared_anchor_tests {
    use super::*;
    use crate::investigate::identity::IdentityPhrase;
    use crate::investigate::r#match::Resolution;

    fn resolution(title: &str) -> Resolution {
        Resolution {
            status: ResolutionStatus::Ambiguous,
            id: None,
            title: None,
            m: 1.0,
            q: 0.8,
            c: 0.8,
            margin: 0.0,
            query: None,
            matched_by: vec![format!("shared_title:{title}")],
        }
    }

    fn evidence(class: &str) -> IdentityEvidence {
        IdentityEvidence {
            pbo: "x.pbo".to_string(),
            phrases: vec![IdentityPhrase {
                text: class.to_string(),
                source: PhraseSource::CfgPatchesClass,
                workshop_id: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn shared_title_removed_when_unanchored() {
        let mut decision = resolution("Gold Squadron");

        prune_shared_title(&mut decision, &evidence("ffaa_data"));

        assert!(decision.matched_by.is_empty());
    }

    #[test]
    fn shared_title_kept_when_anchored() {
        let mut decision = resolution("USP Gear Uniforms");

        prune_shared_title(&mut decision, &evidence("usp_gear_core"));

        assert_eq!(decision.matched_by, ["shared_title:USP Gear Uniforms"]);
    }
}
