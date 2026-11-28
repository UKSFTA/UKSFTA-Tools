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
use super::evidence::{pool_files, PoolBuilder};
use super::http::{LiveHttp, RateLimiter};
use super::identity::{extract_identity, extract_identity_from, IdentityEvidence};
use super::keyed::{KeyedSearch, LiveKeyedSearch};
use super::plan::build_corpus_stats_many;
use super::r#match::{Resolution, ResolutionStatus};
use super::render::{render_all, render_progress, render_summary, summary_of};
use super::resolve::{
    resolve_all, split_fresh, KeyedReport, PboResolution, Progress, ResolveContext, SeenItem,
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
    if let Some(self_id) = self_workshop_id(&caches) {
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
    let manifest_index = WorkshopIndex::load();
    let mut hash_hits: HashMap<String, (String, String, usize)> = HashMap::new();

    // Match each untracked PBO against the index and arbitrate the origin
    for (pbo_name, pbo_path, origin) in &mut results {
        let target_prefix = pbo_prefix(pbo_path);
        let candidates = index.candidates(pbo_name);
        *origin = resolve_pbo_from_index(pbo_path, candidates, target_prefix.as_deref());

        let Some(index) = manifest_index.as_ref() else {
            continue;
        };
        let Some(entries) = index.confirm_named(pbo_path) else {
            continue;
        };
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
            prefix: target_prefix.clone(),
            is_pack,
        });
    }

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
        &examined, &hash_hits, addons_dir, &mod_dirs, use_colour, source,
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

/// Print the online phase: identity resolution and summary.
fn resolve_identities(
    pbos: &[(String, PathBuf)],
    hash_hits: &HashMap<String, (String, String, usize)>,
    addons_dir: &Path,
    mod_dirs: &[ModDir],
    use_colour: bool,
    keyed: Option<&dyn KeyedSearch>,
) -> Result<(), UksftaError> {
    let bikey_names = scan_bikey_names(addons_dir);
    if !bikey_names.is_empty() {
        println!("  Signing keys found: {}", bikey_names.join(", "));
    }

    // The origin phase already hashed every PBO that has a name candidate.
    // A hit needs no browse request and no corpus read, so a pack whose
    // files are all installed can finish without either.
    let mut confirmed: Vec<PboResolution> = Vec::new();
    let mut unresolved: Vec<(String, PathBuf)> = Vec::new();
    for pbo in pbos {
        match hash_hits.get(&pbo.0) {
            Some((id, path, shared)) => {
                confirmed.push(hash_resolution(pbo, id, path, *shared, mod_dirs));
            }
            None => unresolved.push(pbo.clone()),
        }
    }
    if !confirmed.is_empty() {
        println!(
            "  {} PBO(s) confirmed by content hash; no request is spent on them.",
            confirmed.len()
        );
    }
    if unresolved.is_empty() {
        let now = now_secs();
        let rendered = persist(&confirmed, &[], now, Vec::new());
        println!("\n{}", render_all(&rendered, use_colour));
        println!("\n{}", render_summary(&summary_of(&rendered), use_colour));
        return Ok(());
    }
    let pbos = &unresolved;

    println!(
        "\nResolving {} PBO(s) online. The evidence read takes about a minute and \
         Workshop browse requests are paced at one every 4 s, so a run takes minutes.",
        pbos.len()
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

    // One read per PBO serves both the evidence pool and the identity
    // extractor. The read is released before the next PBO, so a large
    // pack is never held in memory at once.
    let examined: HashSet<&Path> = pbos.iter().map(|(_, path)| path.as_path()).collect();
    let mut evidence_by_path: HashMap<PathBuf, IdentityEvidence> = HashMap::new();
    let mut builder = PoolBuilder::new();
    for path in pool_files(addons_dir) {
        let content = PboContent::read(&path);
        builder.add(&content);
        if examined.contains(path.as_path()) {
            evidence_by_path.insert(path.clone(), extract_identity_from(&content));
        }
    }
    let pool = builder.finish();
    let evidence: Vec<_> = pbos
        .iter()
        .map(|(_, path)| match evidence_by_path.remove(path) {
            Some(evidence) => evidence,
            None => extract_identity(path),
        })
        .collect();

    let store = load_store(&store_path());
    let now = now_secs();
    let (kept, pending) = split_fresh(&store, evidence, now);
    if !kept.is_empty() {
        println!(
            "  {} PBO(s) still hold a fresh record; no request is spent on them.",
            kept.len()
        );
    }

    let http = LiveHttp::new();
    let mut limiter = RateLimiter::new(std::thread::sleep);
    let mut ctx = ResolveContext::new(&http, &mut limiter, &corpus, Some(&pool));
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

    let mut all = confirmed;
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
