//! The investigate phase: local origin block, then online resolution.
//!
//! The local block is byte-compatible with the pre-v2 output: it maps each
//! examined PBO to a cached Workshop folder and prints `[UNKNOWN]`,
//! `[RESOLVED]` or `[PACK]`. Only `--online` adds identity resolution, and
//! only a confirmed match prints a Workshop title.
//!
//! Every resolution is written to the identity store, so `--report`
//! prints the same lines offline from the same decisions.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::bikey::scan_bikey_names;
use super::evidence::{EvidencePool, PoolBuilder};
use super::http::{LiveHttp, RateLimiter};
use super::identity::{extract_identity_from, IdentityEvidence};
use super::keyed::{KeyedSearch, LiveKeyedSearch};
use super::plan::build_corpus_stats_many;
use super::r#match::{Resolution, ResolutionStatus};
use super::render::{render_all, render_progress, render_summary, summary_of};
use super::resolve::{
    resolve_all, split_fresh_with, KeyedReport, PboResolution, Progress, ResolveContext, SeenItem,
};
use super::store::{
    load_store, now_secs, record_run, report_resolutions, save_store, store_path, IdentityStore,
};
use super::visibility::check_workshop_visibility;
use crate::error::UksftaError;
use crate::lock::load_lock;
use crate::origin::{
    build_mod_dirs, build_pbo_index, resolve_pbo_from_index, self_workshop_id, ModDir,
    ResolvedOrigin,
};
use crate::pbo::{get_mod_metadata, pbo_fingerprint, pbo_prefix, PboContent};
use crate::pbo_hashes::PboHashes;
use crate::steam::find_all_workshop_caches;
use crate::util::{
    find_pbos, paint, sanitize_for_terminal, stdout_is_tty, workshop_url, C_GREEN, C_RED,
};
use crate::workshop_manifest::WorkshopIndex;

/// Trace the Workshop origin of untracked PBOs in addons/, then, when
/// `online` is set, resolve each PBO's identity and print one line per PBO.
///
/// `report` without `online` renders the stored decisions and needs no
/// network. `report` with `online` resolves, saves, then prints the same
/// lines the store now holds.
pub fn investigate(
    all: bool,
    online: bool,
    report: bool,
    steam_search: bool,
    refresh: bool,
) -> Result<(), UksftaError> {
    if report && !online {
        return investigate_report();
    }

    // A missing or refused key is a configuration fault. It is read before
    // any output, and the keyless path is never used in its place.
    let keyed = if steam_search {
        Some(LiveKeyedSearch::from_env().map_err(|error| UksftaError::Input(error.message()))?)
    } else {
        None
    };

    let caches = find_all_workshop_caches();
    if caches.is_empty() {
        return Err(UksftaError::Input("Workshop cache not found".to_string()));
    }
    let addons_dir = Path::new("addons");
    if !addons_dir.exists() {
        return Err(UksftaError::Input("No addons/ directory".to_string()));
    }

    let mut mod_dirs = build_mod_dirs(&caches);

    // If the current directory is itself a Workshop mod folder (e.g. we are
    // investigating a pack's own contents), exclude it as a candidate.
    let self_id = self_workshop_id(&caches);
    if let Some(self_id) = self_id.as_deref() {
        println!(
            "Investigating Workshop mod {} — excluding it as a candidate.",
            self_id
        );
        mod_dirs.retain(|d| d.id != self_id);
    }

    // Tracked PBO filenames: from mods.lock if present, else all are untracked
    let lock_path = Path::new("mods.lock");
    let lock = load_lock(lock_path)?;
    let tracked: HashSet<String> = lock
        .mods
        .values()
        .flat_map(|m| m.files.iter())
        .filter_map(|f| {
            Path::new(f)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        })
        .collect();

    let mut results: Vec<(String, PathBuf, Option<ResolvedOrigin>)> = Vec::new();

    for pbo in find_pbos(addons_dir) {
        let Some(name) = pbo.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        if !all && tracked.contains(&name) {
            continue;
        }
        results.push((name, pbo, None));
    }

    if results.is_empty() {
        println!("No untracked PBOs in addons/.");
        return Ok(());
    }

    // Build a single-pass index of every cached PBO name to its folders
    let index = build_pbo_index(&mod_dirs);

    // The depot manifest index holds the exact bytes of every installed
    // Workshop file. A name and size find candidates; a hash confirms one.
    // The item under investigation is dropped from it, or every file would
    // confirm against the pack's own manifest.
    let manifest_index = WorkshopIndex::load().map(|mut index| {
        if let Some(self_id) = self_id.as_deref() {
            index.exclude_item(self_id);
        }
        index
    });
    let mut hash_hits: HashMap<String, (String, String, usize)> = HashMap::new();

    // First pass: prefix origin and candidate discovery read no file body.
    let mut target_prefixes: HashMap<String, Option<String>> = HashMap::new();
    for (pbo_name, pbo_path, origin) in &mut results {
        let target_prefix = pbo_prefix(pbo_path);
        let candidates = index.candidates(pbo_name);
        *origin = resolve_pbo_from_index(pbo_path, candidates, target_prefix.as_deref());
        target_prefixes.insert(pbo_name.clone(), target_prefix);
    }

    // One bounded batch hashes every file with a name candidate. An
    // unchanged file is read from the hash cache instead.
    let hash_paths: Vec<PathBuf> = results
        .iter()
        .filter(|(_, path, _)| {
            manifest_index
                .as_ref()
                .is_some_and(|index| !index.candidates_for(path).is_empty())
        })
        .map(|(_, path, _)| path.clone())
        .collect();
    let mut hashes = PboHashes::load(&PboHashes::default_path());
    let digests: HashMap<PathBuf, ([u8; 20], u64)> = hash_paths
        .iter()
        .cloned()
        .zip(hashes.digests(&hash_paths))
        .filter_map(|(path, digest)| digest.map(|digest| (path, digest)))
        .collect();

    for (pbo_name, pbo_path, origin) in &mut results {
        let Some(index) = manifest_index.as_ref() else {
            break;
        };
        let Some((sha, size)) = digests.get(pbo_path) else {
            continue;
        };
        let entries = index.lookup(sha, *size);
        if entries.is_empty() {
            continue;
        }
        let chosen = index
            .pick(entries, origin.as_ref().map(|o| o.id.as_str()))
            .expect("a confirmed match has at least one entry");
        // The prefix still decides whether the file belongs to an
        // aggregate pack, but only while the hash confirms that same item.
        let confirmed_prefix = origin
            .as_ref()
            .is_some_and(|resolved| resolved.id == chosen.published_id);
        let is_pack = confirmed_prefix && origin.as_ref().is_some_and(|resolved| resolved.is_pack);
        hash_hits.insert(
            pbo_name.clone(),
            (
                chosen.published_id.clone(),
                chosen.path.clone(),
                crate::workshop_manifest::distinct_items(entries),
            ),
        );
        *origin = Some(ResolvedOrigin {
            id: chosen.published_id.clone(),
            prefix: target_prefixes.get(pbo_name).cloned().flatten(),
            is_pack,
        });
    }
    hashes.finish();

    // Report
    let use_colour = stdout_is_tty();
    println!("Untracked PBO Investigation:");
    println!("  {} PBO(s) examined", results.len());
    let mut unknown_count = 0;
    for (pbo, _, origin) in &results {
        let Some(origin) = origin else {
            unknown_count += 1;
            println!("  {} {}", paint(use_colour, C_RED, "[UNKNOWN]"), pbo);
            continue;
        };
        let id = &origin.id;
        let meta = mod_dirs
            .iter()
            .find(|d| &d.id == id)
            .map(|d| get_mod_metadata(&d.path))
            .unwrap_or_default();
        let name = if meta.name.is_empty() {
            format!("Mod {}", id)
        } else {
            meta.name
        };
        if origin.is_pack {
            println!(
                "  {} {} -> {} ({}) {} [pack: prefix {} — verify source]",
                paint(use_colour, C_GREEN, "[PACK]"),
                pbo,
                name,
                id,
                workshop_url(id),
                origin.prefix.as_deref().unwrap_or("unknown")
            );
        } else {
            let confirmed = hash_hits
                .get(pbo)
                .map(|(_, path, shared)| {
                    let extra = if *shared > 1 {
                        format!(", shared by {} items", shared)
                    } else {
                        String::new()
                    };
                    format!(" [content hash: {path}{extra}]")
                })
                .unwrap_or_default();
            println!(
                "  {} {} -> {} ({}) {}{}",
                paint(use_colour, C_GREEN, "[RESOLVED]"),
                pbo,
                name,
                id,
                workshop_url(id),
                confirmed
            );
        }
    }
    if unknown_count > 0 {
        println!(
            "\n{} PBO(s) could not be matched to any Workshop cache entry.",
            unknown_count
        );
        println!("They may come from a deleted or private mod, or be manually placed.");
    }

    if !online {
        return Ok(());
    }

    let examined: Vec<(String, PathBuf)> = results
        .iter()
        .map(|(name, path, _)| (name.clone(), path.clone()))
        .collect();
    check_workshop_visibility(&results);
    let source = keyed.as_ref().map(|search| search as &dyn KeyedSearch);
    resolve_identities(
        &examined,
        &hash_hits,
        addons_dir,
        &mod_dirs,
        use_colour,
        source,
        OnlineOptions {
            refresh,
            self_id: self_id.as_deref(),
        },
    )?;
    Ok(())
}

/// Print the status lines from the store, offline.
///
/// The PBO list decides which files are described; the store decides what
/// is said about each. A PBO the store does not describe, or describes
/// under another fingerprint, prints as unresolved and carries no name.
fn investigate_report() -> Result<(), UksftaError> {
    let addons_dir = Path::new("addons");
    if !addons_dir.exists() {
        return Err(UksftaError::Input("No addons/ directory".to_string()));
    }

    let store = load_store(&store_path());
    if store.pbo.is_empty() {
        println!("No cached resolutions. Run 'uksfta investigate --online' first.");
        return Ok(());
    }

    let examined = examined_fingerprints(addons_dir);
    if examined.is_empty() {
        println!("No PBOs in addons/.");
        return Ok(());
    }

    let use_colour = stdout_is_tty();
    let resolutions = report_resolutions(&store, &examined);
    println!("\n{}", render_all(&resolutions, use_colour));
    println!(
        "\n{}",
        render_summary(&summary_of(&resolutions), use_colour)
    );
    Ok(())
}

/// Every PBO in addons/ with its current fingerprint, in file-name order.
fn examined_fingerprints(addons_dir: &Path) -> Vec<(String, String)> {
    let mut examined: Vec<(String, String)> = Vec::new();
    for pbo in find_pbos(addons_dir) {
        let Some(name) = pbo.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let fingerprint = pbo_fingerprint(&pbo).unwrap_or_default();
        examined.push((name, fingerprint));
    }
    examined.sort();
    examined
}

/// The switches one online run needs: whether to drop stored decisions, and
/// the id of the pack being investigated, which is never a candidate.
struct OnlineOptions<'a> {
    refresh: bool,
    self_id: Option<&'a str>,
}

/// Print the online phase: identity resolution and summary.
fn resolve_identities(
    pbos: &[(String, PathBuf)],
    hash_hits: &HashMap<String, (String, String, usize)>,
    addons_dir: &Path,
    mod_dirs: &[ModDir],
    use_colour: bool,
    keyed: Option<&dyn KeyedSearch>,
    options: OnlineOptions<'_>,
) -> Result<(), UksftaError> {
    let OnlineOptions { refresh, self_id } = options;
    let bikey_names = scan_bikey_names(addons_dir);
    if !bikey_names.is_empty() {
        println!("  Signing keys found: {}", bikey_names.join(", "));
    }

    // The origin phase already hashed every PBO that has a name candidate.
    // A hit needs no browse request and no corpus read, so a pack whose
    // files are all installed can finish without either.
    let plan = plan_evidence(pbos, hash_hits, mod_dirs, PboContent::read);
    if !plan.confirmed.is_empty() {
        println!(
            "  {} PBO(s) confirmed by content hash; no request is spent on them.",
            plan.confirmed.len()
        );
    }
    let Some(work) = plan.pending else {
        let rendered = persist(&plan.confirmed, &[], now_secs(), Vec::new());
        println!("\n{}", render_all(&rendered, use_colour));
        println!("\n{}", render_summary(&summary_of(&rendered), use_colour));
        return Ok(());
    };

    println!(
        "\nResolving {} PBO(s) online. The evidence read takes about a minute and \
         Workshop browse requests are paced at one every 4 s, so a run takes minutes.",
        work.pbos.len()
    );

    // Rarity is learned from every installed Workshop PBO, not from this
    // pack alone: a token common across the caches is a family word. A
    // mod directory holds its PBOs in subdirectories, so the corpus is
    // built from the directories that actually hold them.
    let mut corpus_dirs: BTreeSet<PathBuf> = BTreeSet::new();
    corpus_dirs.insert(addons_dir.to_path_buf());
    for pbo in mod_dirs.iter().flat_map(|dir| dir.pbos.iter()) {
        if let Some(parent) = pbo.path.parent() {
            corpus_dirs.insert(parent.to_path_buf());
        }
    }
    let corpus = build_corpus_stats_many(&corpus_dirs.into_iter().collect::<Vec<_>>());

    let store = load_store(&store_path());
    let now = now_secs();
    let (kept, pending) = split_fresh_with(&store, work.evidence, now, refresh);
    if refresh && !store.pbo.is_empty() {
        println!("  --refresh: every stored decision is dropped for this run.");
    }
    if !kept.is_empty() {
        println!(
            "  {} PBO(s) still hold a fresh record; no request is spent on them.",
            kept.len()
        );
    }

    let http = LiveHttp::new();
    let mut limiter = RateLimiter::new(std::thread::sleep);
    let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, Some(&work.pool));
    if let Some(self_id) = self_id {
        ctx.exclude_item(self_id.to_string());
    }
    if let Some(source) = keyed {
        ctx.use_keyed_search(source);
    }
    ctx.schedule_from(&store);
    // The scan is paced, so a long pack would otherwise print nothing for
    // minutes. One line is rewritten in place until the scan ends.
    let scanning = !pending.is_empty();
    if scanning {
        let started = Instant::now();
        // A captured log keeps one line per step. A terminal rewrites the
        // line in place.
        let terminal = std::io::stdout().is_terminal();
        ctx.set_progress(Box::new(move |progress: &Progress| {
            let line = render_progress(
                progress.done,
                progress.total,
                started.elapsed(),
                &progress.label,
            );
            let mut out = std::io::stdout();
            let _ = if terminal {
                write!(out, "\r{line}\x1b[K")
            } else {
                writeln!(out, "{line}")
            };
            let _ = out.flush();
        }));
    }
    let resolutions = resolve_all(pending, &mut ctx);
    if scanning {
        println!();
    }
    // A refused key invalidates every keyed request, so the run stops
    // before its partial result is written or shown as final.
    let keyed_failure = ctx.keyed_failure().cloned();
    let keyed_report = ctx.keyed_report();
    let items = ctx.seen_items();
    drop(ctx);

    if let Some(error) = keyed_failure {
        return Err(UksftaError::Input(error.message()));
    }

    let mut all = plan.confirmed;
    all.extend(resolutions);
    let rendered = persist(&all, &items, now, kept);

    println!("\n{}", render_all(&rendered, use_colour));
    println!("\n{}", render_summary(&summary_of(&rendered), use_colour));
    if !keyed_report.is_empty() {
        print_keyed_report(&keyed_report, &rendered);
    }
    Ok(())
}

/// A decision from the depot manifest index: the PBO's bytes are exactly a
/// file of the item, so no search is needed.
fn hash_resolution(
    pbo: &(String, PathBuf),
    id: &str,
    path: &str,
    shared: usize,
    mod_dirs: &[ModDir],
) -> PboResolution {
    let fingerprint = pbo_fingerprint(&pbo.1).unwrap_or_default();
    let title = mod_dirs
        .iter()
        .find(|dir| dir.id == id)
        .map(|dir| get_mod_metadata(&dir.path).name)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("Mod {id}"));
    let mut matched_by = vec!["manifest_hash".to_string(), format!("path:{path}")];
    if shared > 1 {
        matched_by.push(format!("shared:{shared}"));
    }

    PboResolution {
        pbo: pbo.0.clone(),
        fingerprint: fingerprint.clone(),
        evidence: IdentityEvidence {
            pbo: pbo.0.clone(),
            fingerprint,
            ..Default::default()
        },
        attempts: Vec::new(),
        resolution: Resolution {
            status: ResolutionStatus::Confirmed,
            id: Some(id.to_string()),
            title: Some(title),
            m: 1.0,
            q: 0.0,
            c: 0.0,
            margin: 0.0,
            query: None,
            matched_by,
        },
        candidates: Vec::new(),
    }
}

/// The examined PBOs after the content-hash partition, with the evidence
/// pass for the ones that still need a query.
struct EvidencePlan {
    /// A decision the depot manifest already settled.
    confirmed: Vec<PboResolution>,
    /// `None` when every examined PBO is confirmed. The pool is then not
    /// built, no PBO content is read, and no request is spent.
    pending: Option<PendingEvidence>,
}

/// The PBOs that still need a query, with the evidence read for them.
struct PendingEvidence {
    pbos: Vec<(String, PathBuf)>,
    pool: EvidencePool,
    evidence: Vec<IdentityEvidence>,
}

/// Split the examined PBOs by content-hash confirmation, then read one
/// record per PBO that still needs a query.
///
/// `read` is the only PBO content read on this path. A confirmed PBO is
/// never read: the origin phase hashed its bytes and the manifest names
/// its item. The pool covers the pending PBOs alone, so a mixed pack
/// spends no read on the files the hash already proved, and a run with
/// nothing pending builds no pool at all.
fn plan_evidence(
    pbos: &[(String, PathBuf)],
    hash_hits: &HashMap<String, (String, String, usize)>,
    mod_dirs: &[ModDir],
    mut read: impl FnMut(&Path) -> PboContent,
) -> EvidencePlan {
    let mut confirmed = Vec::new();
    let mut waiting: Vec<(String, PathBuf)> = Vec::new();
    for pbo in pbos {
        match hash_hits.get(&pbo.0) {
            Some((id, path, shared)) => {
                confirmed.push(hash_resolution(pbo, id, path, *shared, mod_dirs));
            }
            None => waiting.push(pbo.clone()),
        }
    }
    if waiting.is_empty() {
        return EvidencePlan {
            confirmed,
            pending: None,
        };
    }
    // Sorted by path, as the whole-pack pool was, so a partial pool keeps
    // the pack's file order.
    waiting.sort();

    // One read per pending PBO serves both the pool and the identity
    // extractor. The read is released before the next PBO, so a large
    // pack is never held in memory at once.
    let mut builder = PoolBuilder::new();
    let mut evidence = Vec::with_capacity(waiting.len());
    for (_, path) in &waiting {
        let content = read(path);
        builder.add(&content);
        evidence.push(extract_identity_from(&content));
    }
    EvidencePlan {
        confirmed,
        pending: Some(PendingEvidence {
            pbos: waiting,
            pool: builder.finish(),
            evidence,
        }),
    }
}

/// The keyed search per PBO: queries sent, ranked candidates, and the
/// identity the evidence rules confirmed. The top hit is never promoted.
fn print_keyed_report(reports: &[KeyedReport], resolutions: &[PboResolution]) {
    println!("\nSteam search candidates (search only; nothing is downloaded):");
    for report in reports {
        println!("  {}", sanitize_for_terminal(&report.pbo));
        let queries = report
            .queries
            .iter()
            .map(|text| format!("\"{}\"", sanitize_for_terminal(text)))
            .collect::<Vec<_>>()
            .join(", ");
        println!("    queries: {queries}");
        if report.ranked.is_empty() {
            println!("    no candidates");
        }
        for (position, (id, title)) in report.ranked.iter().take(KEYED_REPORT_TOP).enumerate() {
            println!(
                "    {}. {} ({})",
                position + 1,
                sanitize_for_terminal(title),
                sanitize_for_terminal(id)
            );
        }
        let confirmed = resolutions.iter().find(|item| {
            item.pbo == report.pbo && item.resolution.status == ResolutionStatus::Confirmed
        });
        match confirmed {
            Some(item) => {
                let title =
                    sanitize_for_terminal(item.resolution.title.as_deref().unwrap_or_default());
                let id = sanitize_for_terminal(item.resolution.id.as_deref().unwrap_or_default());
                println!("    confirmed: \"{title}\" ({id})");
            }
            None => println!("    no identity confirmed by the evidence rules"),
        }
    }
}

/// Ranked keyed candidates shown per PBO.
const KEYED_REPORT_TOP: usize = 5;

/// Merge this run into the store, save it, and return the records the
/// store now holds for exactly these PBOs.
///
/// A record reused from the store is rendered from the store rather than
/// merged again, so its attempt time still dates the query that produced
/// it and an unresolved record keeps its seven-day clock.
fn persist(
    resolutions: &[PboResolution],
    items: &[SeenItem],
    now: u64,
    kept: Vec<PboResolution>,
) -> Vec<PboResolution> {
    let path = store_path();
    let mut store: IdentityStore = load_store(&path);
    let mut rendered = record_run(&mut store, resolutions, items, now);
    rendered.extend(kept);
    rendered.sort_by(|left, right| left.pbo.cmp(&right.pbo));
    if let Err(error) = save_store(&path, &store) {
        eprintln!("Warning: could not save {}: {error}", path.display());
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn pbo(name: &str) -> (String, PathBuf) {
        (name.to_string(), Path::new("addons").join(name))
    }

    fn hits(entries: &[(&str, &str)]) -> HashMap<String, (String, String, usize)> {
        entries
            .iter()
            .map(|(name, id)| {
                (
                    name.to_string(),
                    (id.to_string(), format!("Addons\\{name}"), 1),
                )
            })
            .collect()
    }

    /// A content record with no bytes: enough for the pool and the
    /// clauses that need no packed config.
    fn fake_content(path: &Path) -> PboContent {
        PboContent {
            path: path.to_path_buf(),
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            head: Vec::new(),
            sources: Vec::new(),
            truncated: false,
        }
    }

    #[test]
    fn all_confirmed_plan_builds_no_pool_and_reads_no_pbo() {
        let examined = [pbo("a.pbo"), pbo("b.pbo")];
        let hits = hits(&[("a.pbo", "1"), ("b.pbo", "2")]);
        let reads = Cell::new(0usize);

        let plan = plan_evidence(&examined, &hits, &[], |path| {
            reads.set(reads.get() + 1);
            fake_content(path)
        });

        assert_eq!(reads.get(), 0);
        assert!(plan.pending.is_none());
        assert_eq!(plan.confirmed.len(), 2);
        assert!(plan
            .confirmed
            .iter()
            .all(|item| item.resolution.status == ResolutionStatus::Confirmed));
    }

    #[test]
    fn mixed_plan_builds_the_pool_for_the_unconfirmed_pbo_only() {
        let examined = [pbo("a.pbo"), pbo("miss.pbo"), pbo("b.pbo")];
        let hits = hits(&[("a.pbo", "1"), ("b.pbo", "2")]);
        let reads = Cell::new(0usize);

        let plan = plan_evidence(&examined, &hits, &[], |path| {
            reads.set(reads.get() + 1);
            fake_content(path)
        });

        let pending = plan.pending.expect("a miss keeps the pool");
        assert_eq!(reads.get(), 1);
        assert_eq!(pending.pbos.len(), 1);
        assert_eq!(pending.pbos[0].0, "miss.pbo");
        assert_eq!(pending.evidence.len(), 1);
        assert_eq!(pending.evidence[0].pbo, "miss.pbo");
        assert_eq!(pending.pool.corpus.docs, 1);
    }

    #[test]
    fn settled_lines_render_as_the_store_reports_them() {
        let dir = std::env::temp_dir().join("uksfta-online-settled-lines");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ade_core.pbo");
        std::fs::write(&path, b"hash-confirmed content").unwrap();

        let examined = vec![("ade_core.pbo".to_string(), path.clone())];
        let hits = hits(&[("ade_core.pbo", "123")]);
        let plan = plan_evidence(&examined, &hits, &[], fake_content);
        assert!(plan.pending.is_none());

        let mut store = IdentityStore::empty();
        record_run(&mut store, &plan.confirmed, &[], 1_000);
        let fingerprint = plan.confirmed[0].fingerprint.clone();
        let reported = report_resolutions(&store, &[("ade_core.pbo".to_string(), fingerprint)]);

        assert_eq!(
            render_all(&plan.confirmed, false),
            render_all(&reported, false)
        );
        assert_eq!(
            render_summary(&summary_of(&plan.confirmed), false),
            render_summary(&summary_of(&reported), false)
        );
    }
}
