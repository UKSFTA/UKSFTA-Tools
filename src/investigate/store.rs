//! The v2 identity store: one record per PBO, keyed by file name.
//!
//! The store is the single source of truth for `--report`. A record is
//! reused only while the PBO fingerprint matches, so a changed file is
//! never described by an old decision. Only a confirmed record carries a
//! title. Every collection in the file is ordered, so two runs that make
//! the same decisions write the same bytes.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::identity::IdentityEvidence;
use super::plan::QueryKind;
use super::r#match::{Resolution, ResolutionStatus};
use super::resolve::{PboResolution, QueryAttempt, SeenItem};
use crate::atomic::write_atomic;
use crate::error::UksftaError;

/// Shape of the file. A bump discards old data instead of converting it:
/// a v1 record outlived the ranking that produced it.
pub(crate) const STORE_SCHEMA: u32 = 2;

/// Shape of the ranking behind a record. A bump invalidates the store,
/// because an old score would describe a decision that no longer exists.
pub(crate) const SCORE_VERSION: u32 = 10;

// How long a record may stand before the next resolving run asks again.
// A report always prints a stored line unchanged, whatever its age; only
// the resolving run consults freshness.
pub(crate) const UNRESOLVED_TTL_SECS: u64 = 7 * 24 * 3600;

/// Everything the tool knows across runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct IdentityStore {
    pub schema: u32,
    pub tool: String,
    pub score_version: u32,
    pub updated: u64,
    pub pbo: BTreeMap<String, PboEntry>,
    pub items: BTreeMap<String, ItemEntry>,
}

/// One PBO: the evidence read, the queries tried, and the decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PboEntry {
    pub fingerprint: String,
    pub evidence: IdentityEvidence,
    pub queries: Vec<QueryAttemptRecord>,
    pub resolution: ResolutionRecord,
    /// The strongest candidates the queries ranked, best first. Absent from
    /// a store written before candidates were recorded, and then empty.
    #[serde(default)]
    pub candidates: Vec<CandidateRecord>,
}

/// One candidate near the top of a rank, kept so a report can measure
/// precision at a given rank without querying again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CandidateRecord {
    pub id: String,
    pub title: String,
    pub m: f64,
    pub c: f64,
}

/// One query and what it returned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct QueryAttemptRecord {
    pub text: String,
    pub kind: String,
    pub q: f64,
    pub total_count: Option<u64>,
    pub checked: u64,
    pub outcome: String,
}

/// The decision for one PBO. `title` and `id` are empty unless the
/// status is `confirmed`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ResolutionRecord {
    pub status: String,
    pub id: Option<String>,
    pub title: Option<String>,
    pub m: f64,
    pub q: f64,
    pub c: f64,
    pub margin: f64,
    pub query: Option<String>,
    pub matched_by: Vec<String>,
}

/// A Workshop item seen as a candidate. `result` is the size of the
/// search result set that surfaced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ItemEntry {
    pub title: String,
    pub result: u32,
    pub subs: u64,
    pub checked: u64,
}

impl IdentityStore {
    pub(crate) fn empty() -> Self {
        Self {
            schema: STORE_SCHEMA,
            tool: tool_id(),
            score_version: SCORE_VERSION,
            updated: 0,
            pbo: BTreeMap::new(),
            items: BTreeMap::new(),
        }
    }
}

impl PboEntry {
    /// Record what one run decided about one PBO.
    pub(crate) fn from_resolution(resolution: &PboResolution, now: u64) -> Self {
        Self {
            fingerprint: resolution.fingerprint.clone(),
            evidence: resolution.evidence.clone(),
            queries: resolution
                .attempts
                .iter()
                .map(|attempt| QueryAttemptRecord::from_attempt(attempt, now))
                .collect(),
            resolution: ResolutionRecord::from_resolution(&resolution.resolution),
            candidates: resolution.candidates.clone(),
        }
    }

    /// The same decision, in the shape the resolver and the renderer use.
    pub(crate) fn to_resolution(&self, pbo: &str) -> PboResolution {
        PboResolution {
            pbo: pbo.to_string(),
            fingerprint: self.fingerprint.clone(),
            evidence: self.evidence.clone(),
            attempts: self
                .queries
                .iter()
                .map(QueryAttemptRecord::to_attempt)
                .collect(),
            resolution: self.resolution.to_resolution(),
            candidates: self.candidates.clone(),
        }
    }
}

impl QueryAttemptRecord {
    fn from_attempt(attempt: &QueryAttempt, checked: u64) -> Self {
        Self {
            text: attempt.text.clone(),
            kind: kind_label(attempt.kind).to_string(),
            q: attempt.q,
            total_count: attempt.total_count,
            checked,
            outcome: attempt.outcome.label().to_string(),
        }
    }

    fn to_attempt(&self) -> QueryAttempt {
        QueryAttempt {
            text: self.text.clone(),
            kind: kind_from_label(&self.kind).unwrap_or(QueryKind::IdentityName),
            q: self.q,
            total_count: self.total_count,
            outcome: status_from_label(&self.outcome).unwrap_or(ResolutionStatus::Unresolved),
        }
    }
}

impl ResolutionRecord {
    fn from_resolution(resolution: &Resolution) -> Self {
        let confirmed = resolution.status == ResolutionStatus::Confirmed;
        Self {
            status: resolution.status.label().to_string(),
            id: if confirmed {
                resolution.id.clone()
            } else {
                None
            },
            title: if confirmed {
                resolution.title.clone()
            } else {
                None
            },
            m: resolution.m,
            q: resolution.q,
            c: resolution.c,
            margin: resolution.margin,
            query: resolution.query.clone(),
            matched_by: resolution.matched_by.clone(),
        }
    }

    fn to_resolution(&self) -> Resolution {
        let status = status_from_label(&self.status).unwrap_or(ResolutionStatus::Unresolved);
        let confirmed = status == ResolutionStatus::Confirmed;
        Resolution {
            status,
            id: if confirmed { self.id.clone() } else { None },
            title: if confirmed { self.title.clone() } else { None },
            m: self.m,
            q: self.q,
            c: self.c,
            margin: self.margin,
            query: self.query.clone(),
            matched_by: self.matched_by.clone(),
        }
    }
}

/// The store lives beside the other command caches.
pub(crate) fn store_path() -> PathBuf {
    Path::new(".uksfta").join("identities.json")
}

/// Read the store. A missing file is an empty store. A file from another
/// schema or score version is discarded without conversion, because its
/// decisions came from a ranking this build no longer uses. A malformed
/// file is discarded with one warning line and never panics.
pub(crate) fn load_store(path: &Path) -> IdentityStore {
    let Ok(text) = fs::read_to_string(path) else {
        return IdentityStore::empty();
    };
    match serde_json::from_str::<IdentityStore>(&text) {
        Ok(store) if store.schema == STORE_SCHEMA && store.score_version == SCORE_VERSION => store,
        Ok(_) => IdentityStore::empty(),
        Err(error) => {
            eprintln!(
                "Warning: {}: {error}; discarding the identity store",
                path.display()
            );
            IdentityStore::empty()
        }
    }
}

/// Write the store through the atomic rename, creating `.uksfta/` first.
pub(crate) fn save_store(path: &Path, store: &IdentityStore) -> Result<(), UksftaError> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(store)
        .map_err(|error| UksftaError::Parse(format!("identity store: {error}")))?;
    write_atomic(path, &json)
}

/// Upsert every decision and every candidate item from one run.
pub(crate) fn merge(
    store: &mut IdentityStore,
    resolutions: &[PboResolution],
    items: &[SeenItem],
    now: u64,
) {
    store.schema = STORE_SCHEMA;
    store.tool = tool_id();
    store.score_version = SCORE_VERSION;
    store.updated = now;

    for resolution in resolutions {
        store.pbo.insert(
            resolution.pbo.clone(),
            PboEntry::from_resolution(resolution, now),
        );
    }

    for seen in items {
        let entry = ItemEntry {
            title: seen.item.title.clone(),
            result: seen.total_count,
            subs: seen.item.subscriptions,
            checked: now,
        };
        let newest = store
            .items
            .get(&seen.item.id)
            .is_none_or(|existing| existing.checked <= now);
        if newest {
            store.items.insert(seen.item.id.clone(), entry);
        }
    }
}

/// Record one run: upsert every decision and item, then return the lines
/// the store now holds for exactly these PBOs.
///
/// Both online paths render this result rather than the values in memory.
/// JSON writes a score as the shortest text that identifies it, and the
/// reader can land one unit in the last place away, so rendering the raw
/// values could print a different score from a later `--report`.
pub(crate) fn record_run(
    store: &mut IdentityStore,
    resolutions: &[PboResolution],
    items: &[SeenItem],
    now: u64,
) -> Vec<PboResolution> {
    merge(store, resolutions, items, now);
    let examined: Vec<(String, String)> = resolutions
        .iter()
        .map(|item| (item.pbo.clone(), item.fingerprint.clone()))
        .collect();
    // Render from the text a reader will see. A score that JSON cannot
    // read back exactly would otherwise print differently here than in
    // `--report`, which reads these same bytes from disk.
    let stored = readback(store);
    report_resolutions(&stored, &examined)
}

fn readback(store: &IdentityStore) -> IdentityStore {
    serde_json::to_string(store)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| store.clone())
}

/// Whether a stored decision may stand for the PBO as it is now.
///
/// The fingerprint decides first: a changed file is never described by an
/// old decision. Then the status decides: `rate_limited` and `no_result`
/// retry on the next run, an unresolved record expires after
/// [`UNRESOLVED_TTL_SECS`], and a confirmed record stands for as long as
/// the fingerprint matches.
pub(crate) fn entry_is_fresh(entry: &PboEntry, fingerprint: &str, now: u64) -> bool {
    if fingerprint.is_empty() || entry.fingerprint != fingerprint {
        return false;
    }
    let Some(checked) = entry.queries.iter().map(|query| query.checked).max() else {
        return false;
    };
    if checked == 0 {
        return false;
    }
    !should_retry(&entry.resolution.status, now.saturating_sub(checked))
}

/// When this PBO was last queried: the newest attempt in its stored record,
/// or zero when the store has no attempt for it. The resolver schedules by
/// this value, so a new run starts where the last run stopped.
pub(crate) fn attempted_at(store: &IdentityStore, pbo: &str) -> u64 {
    store
        .pbo
        .get(pbo)
        .and_then(|entry| entry.queries.iter().map(|query| query.checked).max())
        .unwrap_or(0)
}

/// Whether a record with this status and age must be resolved again.
pub(crate) fn should_retry(status: &str, age_secs: u64) -> bool {
    match status {
        "confirmed" | "ambiguous" => false,
        "rate-limited" | "no-result" => true,
        "no-query" | "unresolved" => age_secs >= UNRESOLVED_TTL_SECS,
        _ => true,
    }
}

/// Build one result per examined PBO, in the order given, from the store.
///
/// A PBO that the store does not describe, or describes under another
/// fingerprint, is unresolved: the report never prints a name it cannot
/// prove for the file on disk. No network is used and no reference to a
/// live connection exists on this path.
pub(crate) fn report_resolutions(
    store: &IdentityStore,
    examined: &[(String, String)],
) -> Vec<PboResolution> {
    examined
        .iter()
        .map(|(pbo, fingerprint)| match store.pbo.get(pbo) {
            Some(entry) if !fingerprint.is_empty() && entry.fingerprint == *fingerprint => {
                entry.to_resolution(pbo)
            }
            _ => unresolved(pbo),
        })
        .collect()
}

/// Seconds since the Unix epoch, saturating at zero.
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn unresolved(pbo: &str) -> PboResolution {
    PboResolution {
        pbo: pbo.to_string(),
        fingerprint: String::new(),
        evidence: IdentityEvidence {
            pbo: pbo.to_string(),
            ..Default::default()
        },
        attempts: Vec::new(),
        resolution: Resolution {
            status: ResolutionStatus::Unresolved,
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

fn tool_id() -> String {
    format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

fn kind_label(kind: QueryKind) -> &'static str {
    match kind {
        QueryKind::IdentityName => "identity_name",
        QueryKind::TagPlusHead => "tag_plus_head",
        QueryKind::ClassRootPlusToken => "class_root_plus_token",
    }
}

fn kind_from_label(label: &str) -> Option<QueryKind> {
    match label {
        "identity_name" => Some(QueryKind::IdentityName),
        "tag_plus_head" => Some(QueryKind::TagPlusHead),
        "class_root_plus_token" => Some(QueryKind::ClassRootPlusToken),
        _ => None,
    }
}

fn status_from_label(label: &str) -> Option<ResolutionStatus> {
    match label {
        "confirmed" => Some(ResolutionStatus::Confirmed),
        "ambiguous" => Some(ResolutionStatus::Ambiguous),
        "no-query" => Some(ResolutionStatus::NoDistinctiveQuery),
        "no-result" => Some(ResolutionStatus::NoResult),
        "rate-limited" => Some(ResolutionStatus::RateLimited),
        "unresolved" => Some(ResolutionStatus::Unresolved),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigate::browse::BrowseResult;
    use crate::investigate::identity::{IdentityPhrase, PhraseSource};
    use crate::investigate::render::render_all;

    const TITLE: &str = "ADE - Advanced Diving Environment";
    const DAY: u64 = 24 * 3600;
    const NOW: u64 = 1_700_000_000;

    fn resolution(pbo: &str, status: ResolutionStatus, title: Option<&str>) -> PboResolution {
        PboResolution {
            pbo: pbo.to_string(),
            fingerprint: format!("fp-{pbo}"),
            evidence: IdentityEvidence {
                pbo: pbo.to_string(),
                fingerprint: format!("fp-{pbo}"),
                phrases: vec![IdentityPhrase {
                    text: "ADE diving".to_string(),
                    source: PhraseSource::CfgModsName,
                    workshop_id: None,
                }],
                stem: "ade_core".to_string(),
                ..Default::default()
            },
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

    fn seen(id: &str, title: &str, subs: u64, total_count: u32) -> SeenItem {
        SeenItem {
            item: BrowseResult {
                id: id.to_string(),
                title: title.to_string(),
                short_description: String::new(),
                tags: Vec::new(),
                subscriptions: subs,
                lifetime_subscriptions: subs,
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
            },
            total_count,
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("uksfta-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn examined(pbos: &[&str]) -> Vec<(String, String)> {
        pbos.iter()
            .map(|pbo| ((*pbo).to_string(), format!("fp-{pbo}")))
            .collect()
    }

    /// The store as one run would write it, for the given resolutions.
    fn store_of(resolutions: &[PboResolution], items: &[SeenItem]) -> IdentityStore {
        let mut store = IdentityStore::empty();
        merge(&mut store, resolutions, items, NOW);
        store
    }

    #[test]
    fn store_round_trips() {
        let dir = temp_dir("round-trip");
        let path = dir.join("identities.json");
        let store = store_of(
            &[
                resolution("ade_core.pbo", ResolutionStatus::Confirmed, Some(TITLE)),
                // A non-confirmed record keeps no title even when the caller
                // hands it one: the store is the last guard before the file.
                resolution(
                    "opaque.pbo",
                    ResolutionStatus::Ambiguous,
                    Some("Wrong Name"),
                ),
            ],
            &[seen("1891978580", TITLE, 1200, 200)],
        );

        save_store(&path, &store).unwrap();
        let loaded = load_store(&path);

        assert_eq!(loaded, store);
        assert_eq!(loaded.schema, 2);
        assert_eq!(loaded.score_version, SCORE_VERSION);
        assert_eq!(loaded.items["1891978580"].title, TITLE);
        assert_eq!(loaded.items["1891978580"].result, 200);
        assert_eq!(loaded.items["1891978580"].subs, 1200);
        assert_eq!(
            loaded.pbo["ade_core.pbo"].resolution.title.as_deref(),
            Some(TITLE)
        );
        assert_eq!(loaded.pbo["opaque.pbo"].resolution.title, None);
        assert_eq!(loaded.pbo["opaque.pbo"].resolution.id, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    fn candidate(id: &str, m: f64) -> CandidateRecord {
        CandidateRecord {
            id: id.to_string(),
            title: format!("Title {id}"),
            m,
            c: m,
        }
    }

    #[test]
    fn store_candidate_round_trip() {
        let dir = temp_dir("candidate-round-trip");
        let path = dir.join("identities.json");
        let mut item = resolution("ade_core.pbo", ResolutionStatus::Ambiguous, None);
        item.candidates = vec![candidate("1", 0.90), candidate("2", 0.40)];
        let store = store_of(&[item], &[]);

        save_store(&path, &store).unwrap();
        let loaded = load_store(&path);

        assert_eq!(loaded, store);
        assert_eq!(loaded.pbo["ade_core.pbo"].candidates.len(), 2);
        assert_eq!(loaded.pbo["ade_core.pbo"].candidates[1].id, "2");
        // A row that names nothing still reports the candidates it ranked.
        let reported = report_resolutions(&store, &examined(&["ade_core.pbo"]));
        assert_eq!(reported[0].candidates, store.pbo["ade_core.pbo"].candidates);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn store_loads_schema2_without_candidates() {
        let dir = temp_dir("no-candidates");
        let path = dir.join("identities.json");
        let store = store_of(
            &[resolution(
                "ade_core.pbo",
                ResolutionStatus::Confirmed,
                Some(TITLE),
            )],
            &[],
        );
        // A store written before candidates were recorded has no such key.
        let mut json = serde_json::to_value(&store).unwrap();
        json["pbo"]["ade_core.pbo"]
            .as_object_mut()
            .unwrap()
            .remove("candidates");
        fs::write(&path, serde_json::to_string(&json).unwrap()).unwrap();

        let loaded = load_store(&path);

        assert_eq!(loaded.schema, STORE_SCHEMA);
        assert_eq!(loaded.score_version, SCORE_VERSION);
        assert_eq!(
            loaded.pbo["ade_core.pbo"].resolution.title.as_deref(),
            Some(TITLE)
        );
        assert!(loaded.pbo["ade_core.pbo"].candidates.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn store_discards_v1_schema() {
        let dir = temp_dir("v1");
        let path = dir.join("identities.json");
        fs::write(&path, r#"{"42cdo":[{"id":"1","title":"Wrong Name"}]}"#).unwrap();

        let store = load_store(&path);

        assert_eq!(store.schema, STORE_SCHEMA);
        assert!(store.pbo.is_empty());
        assert!(!serde_json::to_string(&store)
            .unwrap()
            .contains("Wrong Name"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn store_discards_score_version_bump() {
        let dir = temp_dir("score-version");
        let path = dir.join("identities.json");
        let mut store = store_of(
            &[resolution(
                "ade_core.pbo",
                ResolutionStatus::Confirmed,
                Some(TITLE),
            )],
            &[],
        );
        store.score_version = SCORE_VERSION + 1;
        save_store(&path, &store).unwrap();

        let loaded = load_store(&path);

        assert!(loaded.pbo.is_empty());
        assert_eq!(loaded.score_version, SCORE_VERSION);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn store_missing_file_is_empty() {
        let dir = temp_dir("missing");
        let path = dir.join(".uksfta").join("identities.json");

        let store = load_store(&path);

        assert_eq!(store, IdentityStore::empty());
        assert_eq!(store.schema, STORE_SCHEMA);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn store_malformed_file_is_empty() {
        let dir = temp_dir("malformed");
        let path = dir.join("identities.json");
        fs::write(&path, "{\"schema\": 2, \"pbo\": ").unwrap();

        let store = load_store(&path);

        assert_eq!(store, IdentityStore::empty());
        assert_eq!(store.schema, STORE_SCHEMA);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn store_survives_atomic_write() {
        let dir = temp_dir("atomic");
        let path = dir.join("identities.json");
        fs::write(&path, "truncated garbage").unwrap();
        let store = store_of(
            &[resolution(
                "ade_core.pbo",
                ResolutionStatus::Confirmed,
                Some(TITLE),
            )],
            &[],
        );

        save_store(&path, &store).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let parsed: IdentityStore = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed, store);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn report_renders_from_store_without_network() {
        let resolutions = vec![resolution(
            "ade_core.pbo",
            ResolutionStatus::Confirmed,
            Some(TITLE),
        )];
        let store = store_of(&resolutions, &[]);

        let reported = report_resolutions(&store, &examined(&["ade_core.pbo"]));

        assert_eq!(
            render_all(&reported, false),
            "[confirmed] ade_core.pbo -> \"ADE - Advanced Diving Environment\" (1891978580) [query \"ADE diving\", M=1.00 Q=0.83 margin=1.00]"
        );
    }

    #[test]
    fn report_missing_pbo_is_unresolved() {
        let store = store_of(
            &[resolution(
                "ade_core.pbo",
                ResolutionStatus::Confirmed,
                Some(TITLE),
            )],
            &[],
        );

        let reported = report_resolutions(&store, &examined(&["ade_core.pbo", "stranger.pbo"]));

        assert_eq!(reported.len(), 2);
        assert_eq!(reported[1].resolution.status, ResolutionStatus::Unresolved);
        assert_eq!(reported[1].resolution.query, None);
        let line = render_all(&reported[1..], false);
        assert!(line.starts_with("[unresolved] stranger.pbo "));
        assert!(line.contains("no query"));
        assert!(!line.contains(" -> "));
    }

    #[test]
    fn report_fingerprint_mismatch_is_unresolved() {
        let store = store_of(
            &[resolution(
                "ade_core.pbo",
                ResolutionStatus::Confirmed,
                Some(TITLE),
            )],
            &[],
        );

        let reported = report_resolutions(
            &store,
            &[("ade_core.pbo".to_string(), "fp-changed".to_string())],
        );

        assert_eq!(reported[0].resolution.status, ResolutionStatus::Unresolved);
        assert_eq!(reported[0].resolution.title, None);
        assert!(!render_all(&reported, false).contains(TITLE));
    }

    #[test]
    fn store_reuses_confirmed_entry_for_same_fingerprint() {
        let store = store_of(
            &[resolution(
                "ade_core.pbo",
                ResolutionStatus::Confirmed,
                Some(TITLE),
            )],
            &[],
        );
        let entry = &store.pbo["ade_core.pbo"];

        assert!(entry_is_fresh(entry, "fp-ade_core.pbo", NOW + 30 * DAY));
        assert!(!entry_is_fresh(entry, "fp-other.pbo", NOW + 30 * DAY));
        assert!(!entry_is_fresh(entry, "", NOW));

        let reported = report_resolutions(&store, &examined(&["ade_core.pbo"]));
        assert_eq!(reported[0].resolution.status, ResolutionStatus::Confirmed);
        assert_eq!(reported[0].resolution.title.as_deref(), Some(TITLE));
    }

    #[test]
    fn store_retries_rate_limited_next_run() {
        let store = store_of(
            &[resolution(
                "ade_core.pbo",
                ResolutionStatus::RateLimited,
                None,
            )],
            &[],
        );
        let entry = &store.pbo["ade_core.pbo"];

        assert!(should_retry("rate-limited", 0));
        assert!(!entry_is_fresh(entry, "fp-ade_core.pbo", NOW));
        assert!(!entry_is_fresh(entry, "fp-ade_core.pbo", NOW + 30 * DAY));
        assert_eq!(entry.resolution.title, None);
    }

    #[test]
    fn store_retries_unresolved_after_seven_days() {
        let store = store_of(
            &[resolution("opaque.pbo", ResolutionStatus::Unresolved, None)],
            &[],
        );
        let entry = &store.pbo["opaque.pbo"];

        assert!(entry_is_fresh(entry, "fp-opaque.pbo", NOW + 6 * DAY));
        assert!(!entry_is_fresh(entry, "fp-opaque.pbo", NOW + 8 * DAY));
        assert!(!should_retry("unresolved", 6 * DAY));
        assert!(should_retry("unresolved", 8 * DAY));
        assert!(!should_retry("no-query", 6 * DAY));
        assert!(should_retry("no-query", 8 * DAY));
        assert!(!should_retry("confirmed", 400 * DAY));
        assert!(should_retry("something-new", 0));
    }

    #[test]
    fn store_report_matches_online_lines() {
        let mut no_query = resolution("silent.pbo", ResolutionStatus::NoDistinctiveQuery, None);
        no_query.attempts.clear();
        no_query.resolution.query = None;
        no_query.resolution.m = 0.0;
        no_query.resolution.q = 0.0;
        no_query.resolution.margin = 0.0;
        let mut deferred = resolution("late.pbo", ResolutionStatus::Unresolved, None);
        deferred.attempts.clear();
        // This score is the reason the online path renders the stored record:
        // the JSON reader returns the nearest value one unit away, which
        // prints as 0.47 here and as 0.48 from the value in memory.
        let mut tied = resolution("tied.pbo", ResolutionStatus::Ambiguous, None);
        tied.resolution.m = 0.475_000_000_000_000_03;
        let online = vec![
            resolution("ade_core.pbo", ResolutionStatus::Confirmed, Some(TITLE)),
            resolution("bettir.pbo", ResolutionStatus::Ambiguous, None),
            tied,
            no_query,
            deferred,
        ];
        let dir = temp_dir("parity");
        let path = dir.join("identities.json");
        let mut store = IdentityStore::empty();

        let online_lines = render_all(&record_run(&mut store, &online, &[], NOW), false);
        save_store(&path, &store).unwrap();
        let stored = load_store(&path);
        let report_lines = render_all(
            &report_resolutions(
                &stored,
                &examined(&[
                    "ade_core.pbo",
                    "bettir.pbo",
                    "tied.pbo",
                    "silent.pbo",
                    "late.pbo",
                ]),
            ),
            false,
        );

        assert_eq!(online_lines, report_lines);
        assert_eq!(
            online_lines
                .lines()
                .filter(|line| line.starts_with('['))
                .count(),
            5
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
