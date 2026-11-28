//! Verify and repair the private modpack on a game server over SFTP.
//!
//! Only `<root>/@<publishedid>/` is read or written. Dependency tooling
//! (`mod_sources.txt`, `mods.lock`, the Workshop cache) is not used here.

mod cfgfile;
mod config;
mod doctor;
mod hostkey;
mod launch;
mod limit;
mod native;
mod notify;
mod output;
mod parse;
mod plan;
mod progress;
mod report;
mod sftp;
mod signature;
mod summary;
mod verify;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::error::UksftaError;
use crate::lock::{load_lock, LockFile};
use crate::modlist::{parse_mod_sources, ModEntry};
use crate::util::human_size;

use self::config::{Overrides, Settings};
use self::hostkey::HostKeyPolicy;
use self::output::{JsonReport, TimingsJson};
use self::plan::{
    is_addons_pbo, is_addons_signature, is_key_file, is_pbo, join_remote, mod_dir, walk_release,
    Action, LocalEntry, LocalPack, Plan,
};
use self::report::{
    plan_workers, print_plan, print_report, remaining_units, upload_batch, upload_dirs,
    upload_units, worker_lines, UploadUnit,
};
use self::sftp::ScratchDir;
use self::sftp::Sftp;
use self::verify::Verifier;

/// When to show the client's own transfer meter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// A terminal and a single job.
    Auto,
    /// Use the meter path even when the output is redirected.
    Always,
    /// Never show the meter.
    Never,
}

/// The `server` command flags.
pub struct ServerOptions {
    pub config: Option<PathBuf>,
    pub destination: Option<String>,
    pub port: Option<u16>,
    pub root: Option<String>,
    pub identity: Option<PathBuf>,
    pub meta: Option<PathBuf>,
    pub local: Option<PathBuf>,
    pub deep: bool,
    pub upload: bool,
    pub prune: bool,
    pub force_prune: bool,
    pub dry_run: bool,
    pub require_signatures: bool,
    /// Upload attempts after the first. Zero means one attempt.
    pub retries: u8,
    /// Uploads to run at the same time. The default is one per core.
    pub jobs: Option<u8>,
    /// sftp transfer buffer in bytes. The default is 32768.
    pub buffer: Option<u32>,
    /// sftp outstanding requests. The default is 64.
    pub requests: Option<u16>,
    /// known_hosts file for the host key check.
    pub known_hosts: Option<PathBuf>,
    /// Host key policy: yes or accept-new.
    pub host_key_policy: Option<String>,
    /// When to show the client's transfer meter.
    pub progress: Progress,
    /// Write progress as one JSON object per line on standard error.
    pub progress_json: bool,
    /// A local copy of the launch line.
    pub launch_line: Option<PathBuf>,
    /// server.cfg path on the server.
    pub server_cfg: Option<String>,
    /// Fail when an opt-in config check fails.
    pub strict_config: bool,
    /// Use the native segmented transport.
    pub segmented: bool,
    /// Use the native whole-file transport.
    pub native_whole: bool,
    /// Keep the system client even when the probe finds native whole faster.
    pub no_native_whole: bool,
    /// Force the system sftp path for one release.
    pub legacy_sftp: bool,
    /// Read a password from the controlling terminal for the native path.
    pub password_prompt: bool,
    /// Files at or above this size may be segmented.
    pub segment_min: Option<u64>,
    /// Scale the segmented connections between one and the maximum.
    pub segment_adaptive: bool,
    /// Probe the link before segmenting.
    pub segment_probe: Option<bool>,
    /// Fsync every N completed ranges.
    pub segment_fsync_every: Option<u8>,
    /// Preallocate the segmented temp file.
    pub preallocate: Option<bool>,
    /// The upload rate limit text.
    pub limit: Option<String>,
    /// Segmented connections.
    pub segment_connections: Option<u8>,
    /// Segmented chunk size.
    pub segment_chunk: Option<u32>,
    /// Resume policy: auto or off.
    pub segment_resume: Option<String>,
    /// Reconnects per segmented run.
    pub segment_reconnects: Option<u8>,
    /// Send one desktop notification when the run finishes.
    pub notify: bool,
    /// Write one JSON object to standard output.
    pub json: bool,
    /// Run the read-only preflight instead of the checks.
    pub doctor: bool,
}

/// Verify the pack, then upload or prune when asked.
///
/// # Errors
/// Returns `Check` when the pack fails verification, and `Input` or `Parse`
/// for a connection, usage or parse failure.
pub fn run(opts: ServerOptions) -> Result<(), UksftaError> {
    // With both JSON modes the human lines would share stderr with the
    // progress events, so the machine channel wins.
    QUIET.store(opts.json && opts.progress_json, Ordering::Relaxed);
    if opts.doctor {
        return run_doctor(&opts);
    }

    let started = Instant::now();
    let mut outcome = Outcome::default();
    let result = run_verify(&opts, &mut outcome);
    let total_ms = millis(started.elapsed());

    if opts.json {
        let report = JsonReport::new(
            outcome.settings.as_ref(),
            outcome.pack.clone(),
            outcome.plan.as_ref(),
            TimingsJson {
                connect_ms: outcome.connect_ms,
                deep_ms: outcome.deep_ms,
                upload_ms: outcome.upload_ms,
                total_ms,
            },
            outcome
                .segment_choice
                .as_ref()
                .map(|(remote, connections)| (remote.as_str(), *connections)),
            outcome
                .segment
                .as_ref()
                .map(|segment| (segment.remote.as_str(), segment.sha256.as_str())),
            outcome
                .segment
                .as_ref()
                .map(|segment| (segment.max, segment.start, segment.peak)),
            outcome.probe.as_ref(),
            &outcome.sent,
            &result,
        );
        if let Ok(text) = serde_json::to_string(&report) {
            println!("{text}");
        }
    } else {
        eprintln!(
            "timing: connect/list {} ms, deep {} ms, upload {} ms, total {total_ms} ms",
            outcome.connect_ms,
            outcome.deep_ms.unwrap_or(0),
            outcome.upload_ms.unwrap_or(0)
        );
    }
    if opts.notify && !opts.dry_run {
        notify::send(&notify::Finished {
            pack: outcome
                .pack
                .as_ref()
                .map(|(id, name)| (id.as_str(), name.as_str())),
            result: &result,
            files: outcome.sent.len(),
            bytes: outcome.sent.iter().map(|file| file.bytes).sum(),
            seconds: started.elapsed().as_secs_f64(),
        });
    }
    result
}

/// The data the JSON report needs, filled as the phases complete.
#[derive(Default)]
struct Outcome {
    settings: Option<Settings>,
    pack: Option<(String, String)>,
    plan: Option<Plan>,
    connect_ms: u64,
    deep_ms: Option<u64>,
    upload_ms: Option<u64>,
    segment: Option<SegmentOutcome>,
    segment_choice: Option<(String, u8)>,
    probe: Option<native::probe::Decision>,
    sent: Vec<summary::SentFile>,
}

/// What the one segmented file produced.
struct SegmentOutcome {
    remote: String,
    sha256: String,
    max: u8,
    start: u8,
    peak: u8,
}

/// Where the upload loop reports progress and where the segmented result
/// lands.
struct UploadReport<'a> {
    mode: progress::Mode,
    json: bool,
    segment: &'a mut Option<SegmentOutcome>,
    probe: &'a mut Option<native::probe::Decision>,
    sent: &'a mut Vec<summary::SentFile>,
}

const fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// Verify the pack, then upload or prune when asked.
fn run_verify(opts: &ServerOptions, outcome: &mut Outcome) -> Result<(), UksftaError> {
    let settings = config::resolve(opts.config.as_deref(), &overrides(opts))?;
    outcome.settings = Some(settings.clone());

    if opts.password_prompt {
        native::connect::ensure_terminal()?;
    }
    hostkey::preflight(&settings)?;
    let launch = settings
        .launch_line
        .as_deref()
        .map(read_launch_line)
        .transpose()?;

    let before_known_hosts = if settings.host_key_policy == HostKeyPolicy::AcceptNew {
        hostkey::snapshot(&settings.known_hosts)
    } else {
        None
    };

    let meta_path = opts
        .meta
        .clone()
        .unwrap_or_else(|| PathBuf::from("meta.cpp"));
    let local = load_local(&meta_path, opts.local.as_deref())?;
    outcome.pack = Some((local.id.clone(), local.name.clone()));
    let meta_upload = local
        .meta()
        .map_or_else(|| local.meta_source.clone(), |file| file.path.clone());
    let scratch = ScratchDir::new("uksfta-server-")?;
    let locked = locked_pbos()?;
    let cross_checks = cross_checks(&local)?;
    let jobs = opts.jobs.unwrap_or_else(default_jobs);
    let requested = match opts.progress {
        Progress::Always => true,
        Progress::Never => false,
        Progress::Auto => jobs == 1 && std::io::stdout().is_terminal(),
    };
    let mode = progress::resolve(
        requested,
        opts.progress_json,
        std::io::stderr().is_terminal(),
    );
    let verifier = Verifier {
        client: Sftp::new(&settings),
        settings: &settings,
        scratch: &scratch,
        local: &local,
        meta_upload,
        locked: &locked,
        deep: opts.deep,
        force_prune: opts.force_prune,
        require_signatures: opts.require_signatures,
    };

    say(opts.json, &format!("pack {} {}", local.id, local.name));
    say(
        opts.json,
        &format!(
            "server: {}:{} root {}",
            settings.destination, settings.port, settings.root
        ),
    );
    for warning in &cross_checks {
        eprintln!("Warning: {warning}");
    }
    if local.skipped > 0 {
        eprintln!(
            "Warning: {} release file(s) are excluded by name or type",
            local.skipped
        );
    }

    let verify_started = Instant::now();
    let verified = verifier.verify_timed();
    match &verified {
        Ok((_, timing)) => {
            outcome.connect_ms = timing.connect_ms;
            outcome.deep_ms = timing.deep_ms;
        }
        Err(_) => outcome.connect_ms = millis(verify_started.elapsed()),
    }
    let (mut plan, _) = verified?;
    if settings.host_key_policy == HostKeyPolicy::AcceptNew {
        if let Some(notice) = hostkey::acceptance_notice(
            before_known_hosts,
            hostkey::snapshot(&settings.known_hosts),
            &settings.destination,
            &settings.known_hosts,
        ) {
            eprintln!("{notice}");
        }
    }
    let cfg_text = match settings.server_cfg.as_deref() {
        Some(path) => fetch_server_cfg(&verifier.client, &settings.root, path, &scratch)?,
        None => None,
    };
    let (config_lines, config_reasons) =
        config_checks(launch.as_deref(), cfg_text.as_deref(), &local.id);
    for line in &config_lines {
        eprintln!("{line}");
    }

    write_report(&plan, opts.json);
    write_plan(&plan, opts.json);

    let segment = if settings.segment.enabled {
        segment_target(&plan, &settings)
    } else {
        None
    };
    // The probe runs for the whole-file path too, so the choice is made on
    // every upload rather than only when segmentation is on.
    let probe = probe_target(
        &plan,
        &settings,
        native::probe::Options {
            segment: settings.segment.enabled,
            native_whole: !opts.no_native_whole && !opts.native_whole && !opts.legacy_sftp,
        },
    );
    outcome.segment_choice = segment
        .as_ref()
        .map(|choice| (choice.unit.remote.clone(), settings.segment.connections));
    if let Some(choice) = &segment {
        say(
            opts.json,
            &format!(
                "segment: {} {}, {} connections, {} chunks",
                remote_name(&choice.unit.remote),
                human_size(choice.unit.size),
                settings.segment.connections,
                choice.chunks
            ),
        );
    }

    if opts.dry_run {
        say(opts.json, "dry run: no changes made");
    } else if opts.upload {
        let upload_started = Instant::now();
        plan = upload_with_retries(
            &verifier,
            plan,
            opts.retries,
            jobs,
            UploadReport {
                mode,
                json: opts.json,
                segment: &mut outcome.segment,
                probe: &mut outcome.probe,
                sent: &mut outcome.sent,
            },
            segment.as_ref(),
            probe.as_ref(),
            opts.password_prompt,
            opts.native_whole,
        )?;
        outcome.upload_ms = Some(millis(upload_started.elapsed()));
    }

    if opts.prune && !opts.dry_run {
        let removals: Vec<String> = plan.removals().map(remove_path).collect();
        if !removals.is_empty() {
            let mut batch = Vec::new();
            for remote in &removals {
                batch.push(format!("rm {}", self::sftp::quote(remote)?));
            }
            verifier.client.batch(&batch)?;
            for remote in &removals {
                say(opts.json, &format!("removed {remote}"));
            }
        }
    }

    outcome.plan = Some(plan.clone());
    let mut reasons = Vec::new();
    if plan.failed() {
        let problems = plan.problems();
        if problems > 0 {
            reasons.push(format!("{problems} pack problem(s)"));
        }
        if opts.require_signatures && plan.unsigned > 0 {
            reasons.push(format!(
                "{} of {} PBO(s) have no signature",
                plan.unsigned, plan.local_pbos
            ));
        }
    }
    if opts.strict_config {
        reasons.extend(config_reasons);
    }
    if !reasons.is_empty() {
        return Err(UksftaError::Check(reasons.join(", ")));
    }
    Ok(())
}

/// The connection fields the flags carry.
fn overrides(opts: &ServerOptions) -> Overrides {
    Overrides {
        destination: opts.destination.clone(),
        port: opts.port,
        root: opts.root.clone(),
        identity: opts.identity.clone(),
        buffer: opts.buffer,
        requests: opts.requests,
        known_hosts: opts.known_hosts.clone(),
        host_key_policy: opts.host_key_policy.clone(),
        launch_line: opts.launch_line.clone(),
        server_cfg: opts.server_cfg.clone(),
        segment: if opts.legacy_sftp {
            Some(false)
        } else if opts.segmented {
            Some(true)
        } else if opts.native_whole {
            // The whole-file transport replaces the segmented one, so a
            // config that enables segmentation does not claim a file.
            Some(false)
        } else {
            None
        },
        segment_min: opts.segment_min,
        segment_adaptive: opts.segment_adaptive.then_some(true),
        probe: opts.segment_probe,
        fsync_every: opts.segment_fsync_every,
        preallocate: opts.preallocate,
        limit: opts.limit.clone(),
        segment_connections: opts.segment_connections,
        segment_chunk: opts.segment_chunk,
        segment_resume: opts.segment_resume.clone(),
        segment_reconnects: opts.segment_reconnects,
    }
}

/// True when the human lines must stay off stderr.
static QUIET: AtomicBool = AtomicBool::new(false);

/// A report line: standard output in human mode, standard error with
/// `--json`, so the JSON object owns standard output. Suppressed when the
/// progress events own standard error.
fn say(json: bool, text: &str) {
    if QUIET.load(Ordering::Relaxed) {
        return;
    }
    if json {
        eprintln!("{text}");
    } else {
        println!("{text}");
    }
}

fn write_report(plan: &Plan, json: bool) {
    if QUIET.load(Ordering::Relaxed) {
        return;
    }
    if json {
        let mut err = std::io::stderr().lock();
        print_report(plan, &mut err);
    } else {
        let mut out = std::io::stdout().lock();
        print_report(plan, &mut out);
    }
}

fn write_plan(plan: &Plan, json: bool) {
    if QUIET.load(Ordering::Relaxed) {
        return;
    }
    if json {
        let mut err = std::io::stderr().lock();
        print_plan(plan, &mut err);
    } else {
        let mut out = std::io::stdout().lock();
        print_plan(plan, &mut out);
    }
}

/// The read-only preflight. It resolves the settings, checks the key file,
/// lists the pack folder and asks for free space. It never plans, verifies,
/// uploads or creates a directory.
fn run_doctor(opts: &ServerOptions) -> Result<(), UksftaError> {
    let started = Instant::now();
    let settings = config::resolve(opts.config.as_deref(), &overrides(opts))?;
    for line in doctor::settings_lines(&settings) {
        say(opts.json, &line);
    }
    say(opts.json, &doctor::segment_line(&settings));

    let meta_path = opts
        .meta
        .clone()
        .unwrap_or_else(|| PathBuf::from("meta.cpp"));
    let pack = read_pack_identity(&meta_path);
    let pack_dir = pack.as_ref().map(|(id, _)| mod_dir(&settings.root, id));
    if let Some((id, name)) = &pack {
        say(opts.json, &format!("doctor: pack {id} {name}"));
    } else {
        say(
            opts.json,
            &format!(
                "doctor: pack unknown (no meta.cpp at {})",
                meta_path.display()
            ),
        );
    }

    let probe = doctor::probe(&settings, pack_dir.as_deref());
    if settings.segment.enabled {
        say(
            opts.json,
            &doctor::segment_size_line(&settings, probe.largest),
        );
        match native::probe(&settings) {
            Ok(extensions) => say(
                opts.json,
                &format!(
                    "segment: server posix-rename {}, fsync {}",
                    yes_no(extensions.posix_rename),
                    yes_no(extensions.fsync)
                ),
            ),
            Err(error) => say(opts.json, &format!("segment: server check failed: {error}")),
        }
    }
    let mut checks = probe.checks;
    if pack.is_none() {
        checks.push(doctor::Check {
            name: "pack",
            ok: false,
            detail: format!("no meta.cpp at {}", meta_path.display()),
        });
    }
    for check in &checks {
        say(
            opts.json,
            &format!(
                "doctor: {} {}: {}",
                check.name,
                if check.ok { "ok" } else { "failed" },
                check.detail
            ),
        );
    }
    say(
        opts.json,
        &match probe.free_space {
            Some(bytes) => format!("doctor: free space {bytes} bytes"),
            None => "doctor: free space unknown".to_string(),
        },
    );
    say(
        opts.json,
        &format!("doctor: connect {} ms", probe.connect_ms),
    );

    let usable = checks.iter().all(|check| check.ok);
    let result = if usable {
        Ok(())
    } else {
        Err(UksftaError::Input(
            "the server or pack folder is not usable".to_string(),
        ))
    };
    if opts.json {
        let total_ms = millis(started.elapsed());
        let report = output::DoctorReport::new(
            Some(&settings),
            pack.clone(),
            checks
                .iter()
                .map(|check| output::DoctorCheckJson {
                    name: check.name.to_string(),
                    ok: check.ok,
                    detail: check.detail.clone(),
                })
                .collect(),
            probe.free_space,
            probe.connect_ms,
            total_ms,
            &result,
        );
        if let Ok(text) = serde_json::to_string(&report) {
            println!("{text}");
        }
    }
    result
}

/// The pack id and name from `meta.cpp`, without the build output.
fn read_pack_identity(path: &Path) -> Option<(String, String)> {
    let text = fs::read_to_string(path).ok()?;
    let meta = parse::parse_meta_cpp(&text);
    let id = meta.publishedid?;
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let name = meta.name.unwrap_or_else(|| format!("Pack {id}"));
    Some((id, name))
}

/// Read a local copy of the launch line.
///
/// # Errors
/// Returns `Input` when the file is missing, empty or over 64 KiB.
fn read_launch_line(path: &Path) -> Result<String, UksftaError> {
    let text = fs::read_to_string(path)
        .map_err(|_| UksftaError::Input(format!("no launch line at {}", path.display())))?;
    if text.trim().is_empty() {
        return Err(UksftaError::Input(format!(
            "launch line at {} is empty",
            path.display()
        )));
    }
    if text.len() > 64 * 1024 {
        return Err(UksftaError::Input(format!(
            "launch line at {} is over 64 KiB",
            path.display()
        )));
    }
    Ok(text.trim().to_string())
}

/// Fetch the remote `server.cfg` into scratch. A failure is a warning, so
/// the check cannot run.
fn fetch_server_cfg(
    client: &Sftp,
    root: &str,
    path: &str,
    scratch: &ScratchDir,
) -> Result<Option<String>, UksftaError> {
    let remote = if path.starts_with('/') {
        path.to_string()
    } else {
        join_remote(root, path)
    };
    let local = scratch.join("server.cfg");
    let _ = fs::remove_file(&local);
    let batch = vec![format!(
        "-get {} {}",
        self::sftp::quote(&remote)?,
        self::sftp::quote(&local.display().to_string())?
    )];
    match client.batch(&batch) {
        Ok(_) => Ok(fs::read_to_string(&local).ok()),
        Err(e) => {
            eprintln!("Warning: server.cfg: {e}");
            Ok(None)
        }
    }
}

/// The check lines, and the reasons that `--strict-config` turns fatal.
fn config_checks(launch: Option<&str>, cfg: Option<&str>, id: &str) -> (Vec<String>, Vec<String>) {
    let mut lines = Vec::new();
    let mut reasons = Vec::new();

    if let Some(text) = launch {
        let result = launch::check(&launch::parse(text), id);
        let line = format!("launch: {}", result.describe(id));
        if result.failed() {
            reasons.push(line.clone());
        }
        lines.push(line);
    }

    if let Some(text) = cfg {
        for finding in cfgfile::parse(text).check() {
            let line = format!("server.cfg: {}", finding.describe());
            if finding.fatal() {
                reasons.push(line.clone());
            }
            lines.push(line);
        }
    }

    (lines, reasons)
}

fn remove_path(action: &Action) -> String {
    match action {
        Action::Remove { remote } | Action::Upload { remote, .. } => remote.clone(),
    }
}

/// What the upload loop does after one attempt.
#[derive(Debug, PartialEq, Eq)]
enum NextAttempt {
    Stop,
    Retry { wait: Duration },
}

/// The one file the segmented path sends, and the number of ranges.
struct SegmentChoice {
    unit: UploadUnit,
    chunks: usize,
}

/// The largest file at or above the minimum, if one can be segmented.
fn segment_target(plan: &Plan, settings: &Settings) -> Option<SegmentChoice> {
    let unit = upload_units(plan)
        .into_iter()
        .filter(|unit| unit.size >= settings.segment.min_bytes)
        .max_by(|a, b| a.size.cmp(&b.size).then_with(|| b.remote.cmp(&a.remote)))?;
    match native::segment::SegmentPlan::new(
        unit.size,
        settings.segment.connections,
        settings.segment.chunk,
    ) {
        Ok(plan) => Some(SegmentChoice {
            unit,
            chunks: plan.chunks.len(),
        }),
        Err(error) => {
            eprintln!("Warning: segment {}: {error}", remote_name(&unit.remote));
            None
        }
    }
}

/// The last path segment of a remote path.
fn remote_name(remote: &str) -> &str {
    remote.rsplit('/').next().unwrap_or(remote)
}

/// The file the probe measures, and the paths it may choose.
struct ProbeTarget {
    unit: UploadUnit,
    options: native::probe::Options,
}

/// The largest file the probe measures, if any. The probe moves about 12 MiB
/// before the transfer, so a file below the segment minimum or below the
/// probe minimum is not worth it. It never requires `--segmented`, and it is
/// skipped when it has no second path to choose.
fn probe_target(
    plan: &Plan,
    settings: &Settings,
    options: native::probe::Options,
) -> Option<ProbeTarget> {
    if !settings.segment.probe || (!options.segment && !options.native_whole) {
        return None;
    }
    let floor = settings
        .segment
        .min_bytes
        .max(native::probe::MIN_PROBE_BYTES);
    let unit = upload_units(plan)
        .into_iter()
        .filter(|unit| unit.size >= floor)
        .max_by(|a, b| a.size.cmp(&b.size).then_with(|| b.remote.cmp(&a.remote)))?;
    Some(ProbeTarget { unit, options })
}

/// The digest the summary shows for a file that cannot be read.
const MISSING_HASH: &str = "-";

/// The most parallel uploads the flag accepts.
pub const MAX_JOBS: u8 = 8;

/// The parallel uploads when `--jobs` is absent: one per core, at most eight.
pub fn default_jobs() -> u8 {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    u8::try_from(cores).unwrap_or(MAX_JOBS).min(MAX_JOBS)
}

/// The hashing threads: four at most, and two when one worker sends,
/// because a single connection leaves the disk and the link competing.
fn hash_threads(jobs: u8) -> usize {
    let cap = if jobs <= 1 { 2 } else { 4 };
    std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(cap)
}

/// A pool that hashes local files on several threads. The caller may join
/// it whenever it wants the digests, so the hash can run beside the send.
struct LocalHashes {
    results: Arc<Mutex<HashMap<PathBuf, String>>>,
    workers: Vec<JoinHandle<()>>,
}

impl LocalHashes {
    /// Start `threads` workers over `paths`. A repeated path is hashed once
    /// per queue entry, which is cheaper than a de-duplication pass.
    fn start(paths: Vec<PathBuf>, threads: usize) -> Self {
        let queue: Arc<Mutex<VecDeque<PathBuf>>> = Arc::new(Mutex::new(paths.into()));
        let results: Arc<Mutex<HashMap<PathBuf, String>>> = Arc::new(Mutex::new(HashMap::new()));
        let mut workers = Vec::new();
        for _ in 0..threads.max(1) {
            let queue = Arc::clone(&queue);
            let results = Arc::clone(&results);
            workers.push(std::thread::spawn(move || loop {
                let next = {
                    let mut queue = queue.lock().unwrap_or_else(|error| error.into_inner());
                    queue.pop_front()
                };
                let Some(path) = next else { break };
                let digest =
                    crate::util::file_sha256_with_buffer(&path, crate::util::HASH_BUFFER_LARGE)
                        .unwrap_or_else(|| MISSING_HASH.to_string());
                let mut results = results.lock().unwrap_or_else(|error| error.into_inner());
                results.insert(path, digest);
            }));
        }
        Self { results, workers }
    }

    /// Wait for every worker and return the digests. A worker that panicked
    /// leaves its paths out, so they fall back to `-`.
    fn join(self) -> HashMap<PathBuf, String> {
        for worker in self.workers {
            let _ = worker.join();
        }
        let mut guard = self
            .results
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        std::mem::take(&mut *guard)
    }
}

/// A pool that starts on demand. The upload begins it at the first completed
/// file, so the local reads do not compete with the first send.
struct LazyHashes {
    paths: Vec<PathBuf>,
    threads: usize,
    pool: Mutex<Option<LocalHashes>>,
}

impl LazyHashes {
    fn new(paths: Vec<PathBuf>, threads: usize) -> Self {
        Self {
            paths,
            threads,
            pool: Mutex::new(None),
        }
    }

    /// Start the pool once. A second call does nothing.
    fn begin(&self) {
        let mut pool = self.pool.lock().unwrap_or_else(|error| error.into_inner());
        if pool.is_none() {
            *pool = Some(LocalHashes::start(self.paths.clone(), self.threads));
        }
    }

    /// Start the pool if it never started, then collect the digests.
    fn join(self) -> HashMap<PathBuf, String> {
        self.begin();
        self.pool
            .into_inner()
            .unwrap_or_else(|error| error.into_inner())
            .map(LocalHashes::join)
            .unwrap_or_default()
    }
}

/// Send the upload batch, re-verify, and send again only what is still
/// missing or stale. The wait grows with the attempt number.
#[allow(clippy::too_many_arguments)]
fn upload_with_retries(
    verifier: &Verifier<'_>,
    mut plan: Plan,
    retries: u8,
    jobs: u8,
    mut report: UploadReport<'_>,
    segment: Option<&SegmentChoice>,
    probe: Option<&ProbeTarget>,
    password_prompt: bool,
    whole: bool,
) -> Result<Plan, UksftaError> {
    let units = upload_units(&plan);
    let view = report
        .mode
        .is_on()
        .then(|| progress::UploadProgress::new(&units, report.mode));
    // The prompt owns the terminal, so the view is hidden while it runs.
    let password = if password_prompt && (segment.is_some() || whole || probe.is_some()) {
        if let Some(view) = &view {
            view.hide();
        }
        let prompted = native::prompt_password(verifier.settings);
        if let Some(view) = &view {
            view.show();
        }
        Some(prompted?)
    } else {
        None
    };
    let outcome = upload_loop(
        verifier,
        &mut plan,
        retries,
        jobs,
        &mut report,
        segment,
        probe,
        password,
        view.clone(),
        whole,
    );
    if let Some(view) = &view {
        view.finish();
    }
    outcome.map(|()| plan)
}

/// One summary row waiting for its digest. The whole-file path fills the
/// digest from the send; the batch path waits for the hash pool.
struct SentRow {
    unit: UploadUnit,
    seconds: f64,
    hash: Option<String>,
    /// True when the seconds and the digest come from the send.
    exact: bool,
}

#[allow(clippy::too_many_arguments)]
fn upload_loop(
    verifier: &Verifier<'_>,
    plan: &mut Plan,
    retries: u8,
    jobs: u8,
    report: &mut UploadReport<'_>,
    segment: Option<&SegmentChoice>,
    probe: Option<&ProbeTarget>,
    password: Option<Arc<native::secret::Secret>>,
    view: Option<Arc<progress::UploadProgress>>,
    whole: bool,
) -> Result<(), UksftaError> {
    let mut whole = whole;
    let mut use_segment = segment.is_some();
    let upload_started = Instant::now();
    let mut skip: HashSet<String> = HashSet::new();
    // The probe and the native writer open files inside the pack folder, so
    // the directories must exist first. The batch below repeats the mkdir
    // lines, which a batch client tolerates.
    if segment.is_some() || probe.is_some() {
        match upload_dirs(plan) {
            Ok(dirs) if !dirs.is_empty() => {
                if let Err(error) = verifier.client.batch(&dirs) {
                    eprintln!("Warning: cannot create the remote directories: {error}");
                }
            }
            Ok(_) => {}
            Err(error) => eprintln!("Warning: {error}"),
        }
    }

    if let Some(target) = probe {
        match native::probe::run_blocking(
            verifier.settings,
            Path::new("."),
            &verifier.local.id,
            &target.unit,
            password.as_ref(),
            jobs,
            target.options,
        ) {
            Ok(decision) => {
                use_segment = decision.segmented();
                whole = decision.native_whole();
                if decision.probe.system_mbps > 0.0 {
                    say(report.json, &decision.rates_line());
                    say(report.json, &decision.decision_line());
                } else {
                    let keeping = if decision.segmented() {
                        "segmented"
                    } else if decision.native_whole() {
                        "native whole-file"
                    } else {
                        "system"
                    };
                    eprintln!(
                        "Warning: probe: the system path could not authenticate; keeping the {keeping} path"
                    );
                }
                *report.probe = Some(decision);
            }
            Err(UksftaError::Auth(message)) => return Err(UksftaError::Auth(message)),
            Err(error) => {
                let keeping = if use_segment { "segmented" } else { "system" };
                eprintln!("Warning: probe: {error}; keeping the {keeping} path");
            }
        }
    }

    if let Some(choice) = segment {
        if use_segment {
            if let Some(view) = &view {
                view.file_started(&choice.unit.remote);
            }
            let trace = view.as_ref().map(|view| progress::SegmentedTrace {
                progress: Arc::clone(view),
                remote: choice.unit.remote.clone(),
            });
            let name = remote_name(&choice.unit.remote);
            let started = Instant::now();
            match native::run_report(
                verifier.settings,
                Path::new("."),
                &verifier.local.id,
                &choice.unit,
                trace,
                password.clone(),
            ) {
                Ok(run) => {
                    let elapsed = started.elapsed();
                    let hash = run.hash;
                    say(
                    report.json,
                    &format!(
                        "segment: {name} published in {:.1} s, {:.2} MB/s, max {}, start {}, peak {}, sha256 {}",
                        elapsed.as_secs_f64(),
                        transfer_rate(choice.unit.size, elapsed),
                        run.max,
                        run.start,
                        run.peak,
                        hash.get(..12).unwrap_or(&hash)
                    ),
                );
                    if let Some(view) = &view {
                        view.file_done(&choice.unit.remote, Some(&hash));
                    }
                    report.sent.push(summary::SentFile::new(
                        name.to_string(),
                        choice.unit.remote.clone(),
                        choice.unit.size,
                        elapsed.as_secs_f64(),
                        false,
                        hash.clone(),
                    ));
                    *report.segment = Some(SegmentOutcome {
                        remote: choice.unit.remote.clone(),
                        sha256: hash,
                        max: run.max,
                        start: run.start,
                        peak: run.peak,
                    });
                    skip.insert(choice.unit.remote.clone());
                    // The file is on the server. Record it now, because the
                    // batch below may have nothing else to send and would
                    // otherwise leave its finding and its count behind.
                    plan.mark_uploaded(&HashSet::from([choice.unit.remote.clone()]));
                }
                Err(UksftaError::Locked(message)) => {
                    return Err(UksftaError::Locked(message));
                }
                Err(UksftaError::Auth(message)) if password.is_some() => {
                    return Err(UksftaError::Auth(message));
                }
                Err(error) => {
                    eprintln!("{}", progress::fallback_line(&error));
                    if let Some(view) = &view {
                        view.file_failed(&choice.unit.remote);
                    }
                }
            }
        }
    }

    let limit = u16::from(retries) + 1;
    // A published segment is a completed attempt even when the batch is empty.
    let mut attempted = !skip.is_empty();
    let mut sent_files: Vec<SentRow> = Vec::new();
    // The pool hashes beside the batch. It starts when the first file
    // completes, so a fast link is not waiting on four threads of disk. The
    // whole-file path hashes as it sends, so it gives the pool no paths.
    let paths: Vec<PathBuf> = if whole {
        Vec::new()
    } else {
        remaining_units(plan, &skip)
            .into_iter()
            .map(|unit| unit.local)
            .collect()
    };
    let hashes = LazyHashes::new(paths, hash_threads(jobs));
    for attempt in 1..=limit {
        let units: Vec<UploadUnit> = remaining_units(plan, &skip);
        if units.is_empty() {
            break;
        }
        if !whole {
            let batch = upload_batch(plan, &skip)?;
            if batch.is_empty() {
                break;
            }
        }
        attempted = true;
        let files = units.len();
        let bytes: u64 = units.iter().map(|unit| unit.size).sum();
        let started = Instant::now();
        let mut native_rows: Vec<native::whole::SentFile> = Vec::new();
        let clean = if whole {
            let outcome = native::whole::run_batch(
                verifier.settings,
                units.clone(),
                jobs,
                password.clone(),
                view.clone(),
            )?;
            for error in &outcome.errors {
                eprintln!("Warning: upload attempt {attempt}: {error}");
            }
            native_rows = outcome.sent;
            outcome.errors.is_empty()
        } else {
            let clean =
                send_parallel(verifier, plan, jobs, attempt, &skip, view.as_ref(), &hashes)?;
            // A worker that never reported a completion has left the pool to
            // the verify phase instead.
            hashes.begin();
            clean
        };
        if clean {
            // Every batch reported success, so the plan is already what a
            // re-listing would return and the remote walk is not paid again.
            let remotes: HashSet<String> = units.iter().map(|unit| unit.remote.clone()).collect();
            plan.mark_uploaded(&remotes);
        } else {
            *plan = verifier.verify()?;
        }
        let elapsed = started.elapsed();
        let left: HashSet<String> = upload_units(plan)
            .into_iter()
            .map(|unit| unit.remote)
            .collect();
        if whole {
            // Each row carries the digest of its own send, so the file is
            // not read again.
            for file in native_rows {
                if left.contains(&file.unit.remote) {
                    continue;
                }
                sent_files.push(SentRow {
                    unit: file.unit,
                    seconds: file.seconds,
                    hash: Some(file.hash),
                    exact: true,
                });
            }
        } else {
            // A file that is no longer in the plan is on the server. Its
            // seconds are its share of this attempt's bytes, so the row is
            // marked as an estimate.
            let share = if bytes == 0 {
                0.0
            } else {
                elapsed.as_secs_f64() / bytes as f64
            };
            for unit in units {
                if left.contains(&unit.remote) {
                    continue;
                }
                let seconds = unit.size as f64 * share;
                sent_files.push(SentRow {
                    unit,
                    seconds,
                    hash: None,
                    exact: false,
                });
            }
        }
        say(
            report.json,
            &format!(
                "attempt {attempt}: {files} file(s), {} attempted, {:.1} s, {:.2} MB/s",
                human_size(bytes),
                elapsed.as_secs_f64(),
                transfer_rate(bytes, elapsed)
            ),
        );
        match next_attempt(plan, attempt, limit) {
            NextAttempt::Stop => break,
            NextAttempt::Retry { wait } => std::thread::sleep(wait),
        }
    }
    if attempted {
        say(report.json, "after upload:");
        write_report(plan, report.json);
    }
    // The rows are built once the batch has finished. The order is the
    // order the attempts sent the files, so the table does not move.
    let hashes = if whole { HashMap::new() } else { hashes.join() };
    for row in sent_files {
        let SentRow {
            unit,
            seconds,
            hash,
            exact,
        } = row;
        let sha256 = match hash.or_else(|| hashes.get(&unit.local).cloned()) {
            Some(sha256) => sha256,
            // A path the pool did not reach, because a worker panicked or
            // the file arrived on a later attempt, is hashed here instead.
            None => {
                report.sent.push(summary::SentFile::whole(
                    &unit.local,
                    &unit.remote,
                    unit.size,
                    seconds,
                ));
                continue;
            }
        };
        report.sent.push(if exact {
            summary::SentFile::native(&unit.local, &unit.remote, unit.size, seconds, sha256)
        } else {
            summary::SentFile::whole_with_hash(
                &unit.local,
                &unit.remote,
                unit.size,
                seconds,
                sha256,
            )
        });
    }
    if !report.sent.is_empty() {
        let seconds = upload_started.elapsed().as_secs_f64();
        for line in summary::render(report.sent, seconds) {
            say(report.json, &line);
        }
    }
    Ok(())
}

/// Make the directories once, then send every unit in one batch per worker.
/// Each worker builds its own client, so the workers run at the same time.
/// Returns false when any batch reported an error.
fn send_parallel(
    verifier: &Verifier<'_>,
    plan: &Plan,
    jobs: u8,
    attempt: u16,
    skip: &HashSet<String>,
    view: Option<&Arc<progress::UploadProgress>>,
    hashes: &LazyHashes,
) -> Result<bool, UksftaError> {
    let dirs = upload_dirs(plan)?;
    let units = remaining_units(plan, skip);
    let workers = plan_workers(units, jobs);
    let markers = view.is_some();
    // One worker is a serial phase and rides the run's shared connection. Two
    // or more keep a connection each: aggregate throughput on a link with
    // latency comes from the number of streams, not from the handshakes.
    let share = workers.len() == 1;
    let mut clean = true;

    // A single worker also creates the directories in its own batch, so a
    // serial upload is one child for the whole attempt.
    if workers.len() > 1 && !dirs.is_empty() {
        if let Err(e) = verifier.client.batch(&dirs) {
            eprintln!("Warning: upload attempt {attempt}: {e}");
            clean = false;
        }
    }

    let mut batches: Vec<Vec<String>> = Vec::with_capacity(workers.len());
    for worker in &workers {
        let mut lines = worker_lines(worker, markers)?;
        if workers.len() <= 1 && !dirs.is_empty() {
            let mut all = dirs.clone();
            all.append(&mut lines);
            lines = all;
        }
        batches.push(lines);
    }

    let results: Vec<Result<(), UksftaError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = workers
            .iter()
            .zip(&batches)
            .map(|(units, lines)| {
                scope.spawn(move || {
                    upload_worker(verifier.settings, lines, units, view, hashes, share)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(UksftaError::Input(
                        "an upload worker stopped unexpectedly".to_string(),
                    ))
                })
            })
            .collect()
    });
    for (index, result) in results.iter().enumerate() {
        if let Err(e) = result {
            clean = false;
            eprintln!(
                "Warning: upload attempt {attempt} worker {}: {e}",
                index + 1
            );
        }
    }
    Ok(clean)
}

/// One worker: its own client, one batch for all of its units, and the
/// progress rows it updates. Each `pwd` marker names the file that has just
/// completed, so the rows move without a child per file.
///
/// `share` is false when the attempt has several workers, and then each one
/// opens its own connection.
fn upload_worker(
    settings: &Settings,
    lines: &[String],
    units: &[UploadUnit],
    view: Option<&Arc<progress::UploadProgress>>,
    hashes: &LazyHashes,
    share: bool,
) -> Result<(), UksftaError> {
    let client = if share {
        Sftp::new(settings)
    } else {
        Sftp::own_connection(settings)
    };
    let Some(view) = view else {
        return client.batch(lines).map(|_| ());
    };
    let Some(first) = units.first() else {
        return client.batch(lines).map(|_| ());
    };

    view.file_started(&first.remote);
    let mut completed = 0usize;
    let sent = client.batch_tracked(lines, |line| {
        if !parse::is_completion_marker(line) {
            return;
        }
        let Some(unit) = units.get(completed) else {
            return;
        };
        hashes.begin();
        view.advance(&unit.remote, unit.size);
        view.file_done(&unit.remote, None);
        completed += 1;
        if let Some(next) = units.get(completed) {
            view.file_started(&next.remote);
        }
    });
    match sent {
        Ok(_) => Ok(()),
        Err(error) => {
            if let Some(unit) = units.get(completed) {
                view.file_failed(&unit.remote);
            }
            Err(error)
        }
    }
}

/// Stop when every file is in place, or when no attempt remains.
fn next_attempt(plan: &Plan, attempt: u16, limit: u16) -> NextAttempt {
    if plan.uploads().next().is_none() || attempt >= limit {
        NextAttempt::Stop
    } else {
        NextAttempt::Retry {
            wait: Duration::from_secs(u64::from(attempt) * 2),
        }
    }
}

/// Transfer rate in MB/s. An instant attempt reports zero.
fn transfer_rate(bytes: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();
    if seconds <= f64::EPSILON {
        0.0
    } else {
        bytes as f64 / seconds / 1_000_000.0
    }
}

/// `mods.lock` from the repo root, when present. A missing file is not an
/// error: the cross-checks are skipped.
fn load_lock_file() -> Result<Option<LockFile>, UksftaError> {
    let path = Path::new("mods.lock");
    if path.exists() {
        Ok(Some(load_lock(path)?))
    } else {
        Ok(None)
    }
}

fn load_sources() -> Result<Option<Vec<ModEntry>>, UksftaError> {
    let path = Path::new("mod_sources.txt");
    if path.exists() {
        Ok(Some(parse_mod_sources(path)?.0))
    } else {
        Ok(None)
    }
}

/// Lowercase basenames of every PBO in `mods.lock`.
fn locked_pbos() -> Result<HashSet<String>, UksftaError> {
    let Some(lock) = load_lock_file()? else {
        return Ok(HashSet::new());
    };
    Ok(lock
        .mods
        .values()
        .flat_map(|entry| entry.files.iter())
        .filter_map(|path| Path::new(path).file_name().and_then(|name| name.to_str()))
        .filter(|name| is_pbo(name))
        .map(|name| name.to_ascii_lowercase())
        .collect())
}

/// Read-only cross-checks against the dependency tooling. Missing files are
/// skipped, and every message is a warning.
fn cross_checks(local: &LocalPack) -> Result<Vec<String>, UksftaError> {
    let lock = load_lock_file()?;
    let sources = load_sources()?;
    Ok(cross_check_messages(
        lock.as_ref(),
        sources.as_deref(),
        local,
    ))
}

fn cross_check_messages(
    lock: Option<&LockFile>,
    sources: Option<&[ModEntry]>,
    local: &LocalPack,
) -> Vec<String> {
    let Some(lock) = lock else {
        return Vec::new();
    };
    let mut warnings = Vec::new();

    let release: HashSet<String> = local
        .pbos()
        .map(|pbo| basename(&pbo.rel).to_ascii_lowercase())
        .collect();
    let mut locked: Vec<(String, String)> = lock
        .mods
        .values()
        .flat_map(|entry| entry.files.iter())
        .filter_map(|path| {
            let name = Path::new(path).file_name()?.to_str()?;
            is_pbo(name).then(|| (name.to_ascii_lowercase(), name.to_string()))
        })
        .collect();
    locked.sort();
    locked.dedup();
    for (key, name) in locked {
        if !release.contains(&key) {
            warnings.push(format!("{name}: locked PBO is not in the local release"));
        }
    }

    if let Some(sources) = sources {
        for entry in sources {
            if !lock.mods.contains_key(&entry.id) {
                warnings.push(format!(
                    "{}: enabled source mod has no mods.lock entry",
                    entry.id
                ));
            }
        }
    }
    warnings
}

/// Read the pack identity and the local build output.
///
/// # Errors
/// Returns `Input` when `meta.cpp` or the build output is missing, and
/// `Parse` when `meta.cpp` has no numeric `publishedid`.
fn load_local(meta_path: &Path, local: Option<&Path>) -> Result<LocalPack, UksftaError> {
    let text = fs::read_to_string(meta_path).map_err(|_| {
        UksftaError::Input(format!(
            "no meta.cpp at {}\nRun from the pack repo root, or pass --meta PATH",
            meta_path.display()
        ))
    })?;
    let meta = parse::parse_meta_cpp(&text);
    let Some(id) = meta.publishedid else {
        return Err(UksftaError::Parse(format!(
            "{} has no publishedid",
            meta_path.display()
        )));
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(UksftaError::Parse(format!(
            "publishedid in {} is not numeric: {id}",
            meta_path.display()
        )));
    }
    let name = meta.name.unwrap_or_else(|| format!("Pack {id}"));

    let dir = local.map_or_else(default_local_dir, Path::to_path_buf);
    let addons = dir.join("addons");
    if !addons.is_dir() {
        return Err(UksftaError::Input(format!(
            "no addons directory in {}\nBuild the pack first, or pass --local PATH",
            dir.display()
        )));
    }
    let walk = walk_release(&dir)?;
    if !walk.files.iter().any(|file| is_addons_pbo(&file.rel)) {
        return Err(UksftaError::Input(format!(
            "no PBOs in {}",
            addons.display()
        )));
    }
    let (signature_authorities, key_authorities, authority_warnings) =
        read_authorities(&walk.files);

    Ok(LocalPack {
        id,
        name,
        dir,
        files: walk.files,
        dirs: walk.dirs,
        skipped: walk.skipped,
        meta_source: meta_path.to_path_buf(),
        signature_authorities,
        key_authorities,
        authority_warnings,
    })
}

/// Read the embedded authority of every local signature and key. A file
/// that does not parse is a warning and stays unknown.
fn read_authorities(
    files: &[LocalEntry],
) -> (HashMap<String, String>, HashSet<String>, Vec<String>) {
    let mut signature_authorities = HashMap::new();
    let mut warnings = Vec::new();
    for file in files.iter().filter(|file| is_addons_signature(&file.rel)) {
        let name = basename(&file.rel);
        match signature::read_key_file(&file.path) {
            Ok(key) => {
                if let Some(named) = signature_authority(name) {
                    if named != key.name {
                        warnings.push(format!(
                            "{name}: embedded authority is {}, not {named}",
                            key.name
                        ));
                    }
                }
                signature_authorities.insert(name.to_string(), key.name);
            }
            Err(e) => warnings.push(format!("{name}: {e}")),
        }
    }

    let mut key_authorities = HashSet::new();
    for file in files.iter().filter(|file| is_key_file(&file.rel)) {
        let name = basename(&file.rel);
        match signature::read_key_file(&file.path) {
            Ok(key) => {
                if !key_base(name).eq_ignore_ascii_case(&key.name) {
                    warnings.push(format!("{name}: embedded authority is {}", key.name));
                }
                key_authorities.insert(key.name);
            }
            Err(e) => warnings.push(format!("{name}: {e}")),
        }
    }
    (signature_authorities, key_authorities, warnings)
}

/// The authority segment of `name.pbo.<authority>.bisign`.
fn signature_authority(name: &str) -> Option<&str> {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".bisign")?;
    let pos = stem.rfind(".pbo.")?;
    Some(&name[pos + 5..name.len() - ".bisign".len()])
}

fn basename(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}

fn key_base(name: &str) -> &str {
    if name.len() > 6 {
        &name[..name.len() - 6]
    } else {
        name
    }
}

/// `.hemttout/release` when it exists, else `.hemttout/build`.
fn default_local_dir() -> PathBuf {
    let release = PathBuf::from(".hemttout/release");
    if release.is_dir() {
        release
    } else {
        PathBuf::from(".hemttout/build")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::ModLockEntry;
    use std::collections::HashMap;

    const META: &str =
        "protocol = 1;\npublishedid = 3662499431;\nname = \"UKSF Taskforce Alpha Maps\";\n";

    fn pack_dir(root: &Path) -> PathBuf {
        let dir = root.join(".hemttout/release");
        fs::create_dir_all(dir.join("addons")).unwrap();
        fs::create_dir_all(dir.join("keys")).unwrap();
        fs::write(dir.join("addons/uksfta_main.pbo"), b"pbo").unwrap();
        fs::write(dir.join("addons/uksfta_main.pbo.key.bisign"), b"sig").unwrap();
        fs::write(dir.join("keys/uksfta.bikey"), b"key").unwrap();
        fs::write(dir.join("meta.cpp"), META).unwrap();
        fs::write(dir.join("mod.cpp"), b"mod").unwrap();
        dir
    }

    #[test]
    fn load_local_reads_the_pack_identity_and_build_output() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = pack_dir(root.path());
        let meta = root.path().join(".hemttout/release/meta.cpp");

        let local = load_local(&meta, Some(&dir)).unwrap();
        assert_eq!(local.id, "3662499431");
        assert_eq!(local.name, "UKSF Taskforce Alpha Maps");
        assert_eq!(local.pbos().count(), 1);
        assert_eq!(local.signatures().count(), 1);
        assert_eq!(local.keys().count(), 1);
        assert!(local.meta().is_some());
        assert!(local.mod_cpp().is_some());
    }

    fn lock_with(files: &[&str]) -> LockFile {
        let mut mods = HashMap::new();
        mods.insert(
            "450814997".to_string(),
            ModLockEntry {
                files: files.iter().map(|file| file.to_string()).collect(),
                name: "CBA_A3".to_string(),
                tags: Vec::new(),
                dependencies: Vec::new(),
                updated: "0".to_string(),
            },
        );
        LockFile { version: 1, mods }
    }

    fn source(id: &str) -> ModEntry {
        ModEntry {
            id: id.to_string(),
            name: "Mod".to_string(),
            tags: Vec::new(),
            role: "mod".to_string(),
            enabled: true,
            dependencies: Vec::new(),
        }
    }

    fn local_fixture() -> (tempfile::TempDir, LocalPack) {
        let root = tempfile::TempDir::new().unwrap();
        let dir = pack_dir(root.path());
        let meta = root.path().join(".hemttout/release/meta.cpp");
        let local = load_local(&meta, Some(&dir)).unwrap();
        (root, local)
    }

    fn plan_with_uploads(sizes: &[u64]) -> Plan {
        let mut plan = Plan::default();
        for size in sizes {
            plan.actions.push(Action::Upload {
                name: "file".to_string(),
                local: PathBuf::from("/local/file"),
                remote: "/remote/file".to_string(),
                size: *size,
            });
        }
        plan
    }

    #[test]
    fn an_attempt_waits_twice_its_number_before_the_next() {
        let plan = plan_with_uploads(&[10]);

        assert_eq!(
            next_attempt(&plan, 1, 3),
            NextAttempt::Retry {
                wait: Duration::from_secs(2)
            }
        );
        assert_eq!(
            next_attempt(&plan, 2, 3),
            NextAttempt::Retry {
                wait: Duration::from_secs(4)
            }
        );
    }

    #[test]
    fn the_last_attempt_does_not_retry() {
        let plan = plan_with_uploads(&[10]);
        assert_eq!(next_attempt(&plan, 3, 3), NextAttempt::Stop);
    }

    #[test]
    fn a_plan_with_no_uploads_stops_at_once() {
        let plan = plan_with_uploads(&[]);
        assert_eq!(next_attempt(&plan, 1, 3), NextAttempt::Stop);
    }

    #[test]
    fn the_transfer_rate_handles_an_instant_attempt() {
        assert_eq!(transfer_rate(1_000_000, Duration::ZERO), 0.0);
        let rate = transfer_rate(2_000_000, Duration::from_secs(2));
        assert!((rate - 1.0).abs() < f64::EPSILON, "{rate}");
    }

    #[test]
    fn the_launch_line_reader_refuses_missing_empty_and_large_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("missing.txt");
        assert!(matches!(
            read_launch_line(&missing),
            Err(UksftaError::Input(_))
        ));

        let empty = dir.path().join("empty.txt");
        fs::write(&empty, b"  \n").unwrap();
        assert!(matches!(
            read_launch_line(&empty),
            Err(UksftaError::Input(_))
        ));

        let large = dir.path().join("large.txt");
        fs::write(&large, "a".repeat(64 * 1024 + 1)).unwrap();
        assert!(matches!(
            read_launch_line(&large),
            Err(UksftaError::Input(_))
        ));

        let small = dir.path().join("small.txt");
        fs::write(&small, b"-mod=@1\n").unwrap();
        assert_eq!(read_launch_line(&small).unwrap(), "-mod=@1");
    }

    #[test]
    fn load_local_reads_the_signature_and_key_authorities() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = pack_dir(root.path());
        let meta = root.path().join(".hemttout/release/meta.cpp");

        let release = dir.clone();
        fs::remove_file(release.join("addons/uksfta_main.pbo.key.bisign")).unwrap();
        let mut signature = synthetic_key("uksfta_1.0.1", true);
        signature.extend_from_slice(&[0, 0, 0, 0]);
        fs::write(
            release.join("addons/uksfta_main.pbo.uksfta_1.0.1.bisign"),
            &signature,
        )
        .unwrap();
        fs::write(
            release.join("keys/uksfta_1.0.1.bikey"),
            synthetic_key("uksfta_1.0.1", true),
        )
        .unwrap();
        fs::remove_file(release.join("keys/uksfta.bikey")).unwrap();

        let local = load_local(&meta, Some(&dir)).unwrap();
        assert_eq!(
            local
                .signature_authorities
                .get("uksfta_main.pbo.uksfta_1.0.1.bisign")
                .map(String::as_str),
            Some("uksfta_1.0.1")
        );
        assert!(local.key_authorities.contains("uksfta_1.0.1"));
        assert!(
            local.authority_warnings.is_empty(),
            "{:?}",
            local.authority_warnings
        );

        fs::rename(
            release.join("keys/uksfta_1.0.1.bikey"),
            release.join("keys/renamed.bikey"),
        )
        .unwrap();
        let local = load_local(&meta, Some(&dir)).unwrap();
        assert!(local
            .authority_warnings
            .iter()
            .any(|warning| warning.contains("renamed.bikey: embedded authority is uksfta_1.0.1")));
        assert!(local.key_authorities.contains("uksfta_1.0.1"));
    }

    fn synthetic_key(authority: &str, public: bool) -> Vec<u8> {
        let bits: u32 = 1024;
        let blob: u32 = if public { 20 + bits / 8 } else { 596 };
        let mut bytes = Vec::new();
        bytes.extend_from_slice(authority.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&blob.to_le_bytes());
        bytes.push(if public { 6 } else { 7 });
        bytes.push(2);
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&0x2400u32.to_le_bytes());
        bytes.extend_from_slice(if public { b"RSA1" } else { b"RSA2" });
        bytes.extend_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(&65537u32.to_le_bytes());
        bytes.resize(bytes.len() + (blob - 20) as usize, 0);
        bytes
    }

    #[test]
    fn a_locked_pbo_missing_from_the_release_warns() {
        let (_root, local) = local_fixture();
        let lock = lock_with(&["addons/uksfta_main.pbo", "addons/absent.pbo"]);

        let warnings = cross_check_messages(Some(&lock), None, &local);
        assert_eq!(
            warnings,
            vec!["absent.pbo: locked PBO is not in the local release".to_string()]
        );
    }

    #[test]
    fn an_enabled_source_without_a_lock_entry_warns() {
        let (_root, local) = local_fixture();
        let lock = lock_with(&["addons/uksfta_main.pbo"]);
        let sources = vec![source("450814997"), source("1111111111")];

        let warnings = cross_check_messages(Some(&lock), Some(&sources), &local);
        assert_eq!(
            warnings,
            vec!["1111111111: enabled source mod has no mods.lock entry".to_string()]
        );
    }

    #[test]
    fn cross_checks_are_silent_without_the_lock() {
        let (_root, local) = local_fixture();
        let sources = vec![source("1111111111")];

        assert!(cross_check_messages(None, Some(&sources), &local).is_empty());
        assert!(cross_check_messages(None, None, &local).is_empty());
    }

    #[test]
    fn cross_checks_are_silent_when_the_lock_matches() {
        let (_root, local) = local_fixture();
        let lock = lock_with(&["addons/uksfta_main.pbo"]);

        assert!(cross_check_messages(Some(&lock), None, &local).is_empty());
    }

    #[test]
    fn load_local_rejects_a_missing_meta_cpp() {
        let root = tempfile::TempDir::new().unwrap();
        let err = load_local(&root.path().join("meta.cpp"), None).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
    }

    #[test]
    fn load_local_rejects_a_non_numeric_id() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = pack_dir(root.path());
        let meta = root.path().join("meta.cpp");
        fs::write(&meta, "publishedid = \"pack\";\n").unwrap();
        let err = load_local(&meta, Some(&dir)).unwrap_err();
        assert!(matches!(err, UksftaError::Parse(_)));
    }

    #[test]
    fn load_local_requires_an_addons_directory_with_pbos() {
        let root = tempfile::TempDir::new().unwrap();
        let dir = root.path().join("build");
        fs::create_dir_all(&dir).unwrap();
        let meta = root.path().join("meta.cpp");
        fs::write(&meta, META).unwrap();

        let err = load_local(&meta, Some(&dir)).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));

        fs::create_dir_all(dir.join("addons")).unwrap();
        let err = load_local(&meta, Some(&dir)).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
    }

    fn segmented(min_bytes: u64, chunk: Option<u32>) -> Settings {
        let overrides = Overrides {
            destination: Some("user@host".to_string()),
            root: Some("/srv".to_string()),
            ..Overrides::default()
        };
        let mut settings = config::resolve(None, &overrides).unwrap();
        settings.segment = config::Segment {
            enabled: true,
            min_bytes,
            connections: 4,
            chunk,
            ..config::Segment::default()
        };
        settings
    }

    fn upload(name: &str, size: u64) -> Action {
        Action::Upload {
            name: name.to_string(),
            local: PathBuf::from(format!("/local/{name}")),
            remote: format!("/srv/@1/addons/{name}"),
            size,
        }
    }

    fn plan_with(actions: Vec<Action>) -> Plan {
        Plan {
            remote_dir: "/srv/@1".to_string(),
            actions,
            ..Plan::default()
        }
    }

    #[test]
    fn the_segment_target_is_the_largest_file_above_the_minimum() {
        let plan = plan_with(vec![
            upload("small.pbo", 1024 * 1024),
            upload("big.pbo", 16 * 1024 * 1024),
            upload("mid.pbo", 8 * 1024 * 1024),
        ]);
        let settings = segmented(8 * 1024 * 1024, Some(4 * 1024 * 1024));

        let choice = segment_target(&plan, &settings).expect("a target");
        assert_eq!(remote_name(&choice.unit.remote), "big.pbo");
        assert_eq!(choice.chunks, 4);
    }

    #[test]
    fn no_file_below_the_minimum_is_segmented() {
        let plan = plan_with(vec![upload("small.pbo", 1024 * 1024)]);
        let settings = segmented(8 * 1024 * 1024, None);
        assert!(segment_target(&plan, &settings).is_none());
    }

    #[test]
    fn a_tie_picks_the_first_remote_name() {
        let plan = plan_with(vec![
            upload("b.pbo", 8 * 1024 * 1024),
            upload("a.pbo", 8 * 1024 * 1024),
        ]);
        let settings = segmented(1, Some(4 * 1024 * 1024));

        let choice = segment_target(&plan, &settings).expect("a target");
        assert_eq!(remote_name(&choice.unit.remote), "a.pbo");
    }

    const MIB: u64 = 1024 * 1024;
    const PROBE_FLOOR: u64 = native::probe::MIN_PROBE_BYTES;

    fn whole_file(min_bytes: u64) -> Settings {
        let overrides = Overrides {
            destination: Some("user@host".to_string()),
            root: Some("/srv".to_string()),
            ..Overrides::default()
        };
        let mut settings = config::resolve(None, &overrides).unwrap();
        settings.segment = config::Segment {
            enabled: false,
            min_bytes,
            ..config::Segment::default()
        };
        settings
    }

    fn whole_choices() -> native::probe::Options {
        native::probe::Options {
            segment: false,
            native_whole: true,
        }
    }

    #[test]
    fn the_probe_targets_the_largest_file_without_segmentation() {
        let plan = plan_with(vec![
            upload("small.pbo", 8 * MIB),
            upload("big.pbo", PROBE_FLOOR + 64 * MIB),
            upload("mid.pbo", PROBE_FLOOR),
        ]);
        let settings = whole_file(64 * MIB);

        let target = probe_target(&plan, &settings, whole_choices()).expect("a target");
        assert_eq!(remote_name(&target.unit.remote), "big.pbo");
        assert!(!target.options.segment);
        assert!(target.options.native_whole);
    }

    #[test]
    fn a_disabled_probe_has_no_target() {
        let plan = plan_with(vec![upload("big.pbo", PROBE_FLOOR)]);
        let mut settings = whole_file(64 * MIB);
        settings.segment.probe = false;
        assert!(probe_target(&plan, &settings, whole_choices()).is_none());
    }

    #[test]
    fn a_probe_without_a_second_path_has_no_target() {
        let plan = plan_with(vec![upload("big.pbo", PROBE_FLOOR)]);
        let settings = whole_file(64 * MIB);
        let forced = native::probe::Options {
            segment: false,
            native_whole: false,
        };
        assert!(probe_target(&plan, &settings, forced).is_none());
    }

    #[test]
    fn a_small_pack_has_no_target() {
        // The largest file clears the segment minimum but not the probe
        // minimum, so the 12 MiB measurement would cost more than it saves.
        let plan = plan_with(vec![upload("single.pbo", 100 * MIB)]);
        let settings = whole_file(64 * MIB);
        assert!(probe_target(&plan, &settings, whole_choices()).is_none());
    }

    #[test]
    fn a_raised_segment_minimum_skips_the_probe() {
        let plan = plan_with(vec![upload("big.pbo", PROBE_FLOOR)]);
        let settings = whole_file(2 * PROBE_FLOOR);
        assert!(probe_target(&plan, &settings, whole_choices()).is_none());
    }

    #[test]
    fn the_hash_pool_matches_a_direct_digest() {
        let dir = tempfile::TempDir::new().unwrap();
        let paths: Vec<PathBuf> = (0..3)
            .map(|index| {
                let path = dir.path().join(format!("{index}.pbo"));
                fs::write(&path, vec![b'a' + index; 4096 + index as usize]).unwrap();
                path
            })
            .collect();

        let hashes = LocalHashes::start(paths.clone(), 3).join();

        for path in &paths {
            assert_eq!(
                hashes.get(path),
                crate::util::file_sha256(path).as_ref(),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn the_hash_pool_marks_a_missing_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("gone.pbo");

        let hashes = LocalHashes::start(vec![missing.clone()], 1).join();

        assert_eq!(hashes.get(&missing).map(String::as_str), Some(MISSING_HASH));
    }

    #[test]
    fn a_lazy_pool_waits_for_the_first_completion_and_still_hashes() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("lazy.pbo");
        fs::write(&path, b"lazy digest").unwrap();
        let hashes = LazyHashes::new(vec![path.clone()], 1);
        assert!(
            hashes
                .pool
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_none(),
            "no worker has started"
        );

        hashes.begin();
        assert!(hashes
            .pool
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some());

        assert_eq!(
            hashes.join().get(&path),
            crate::util::file_sha256(&path).as_ref(),
            "a started pool returns the same digest"
        );
    }

    #[test]
    fn a_lazy_pool_that_never_started_hashes_on_join() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("late.pbo");
        fs::write(&path, b"late digest").unwrap();

        let hashes = LazyHashes::new(vec![path.clone()], 1).join();

        assert_eq!(hashes.get(&path), crate::util::file_sha256(&path).as_ref());
    }

    #[test]
    fn the_default_worker_count_follows_the_cores() {
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        let expected = u8::try_from(cores).unwrap_or(MAX_JOBS).min(MAX_JOBS);
        assert_eq!(default_jobs(), expected);
        assert!((1..=MAX_JOBS).contains(&default_jobs()));
    }

    #[test]
    fn a_single_worker_caps_the_hash_pool_at_two() {        assert!(hash_threads(1) >= 1);
        assert!(
            hash_threads(1) <= 2,
            "one worker must not start four threads"
        );
        assert!(hash_threads(0) <= 2);
        assert_eq!(hash_threads(1), hash_threads(8).min(2));
    }

    #[test]
    fn a_pooled_digest_reaches_the_table_and_the_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pooled.pbo");
        fs::write(&path, b"pooled digest").unwrap();
        let remote = "/srv/@1/addons/pooled.pbo".to_string();
        let digest = LocalHashes::start(vec![path.clone()], 1)
            .join()
            .remove(&path)
            .expect("a digest");
        assert_eq!(digest.len(), 64);

        let row = summary::SentFile::whole_with_hash(&path, &remote, 13, 1.0, digest.clone());
        let lines = summary::render(std::slice::from_ref(&row), 1.0);
        assert!(lines[2].contains(&digest), "{}", lines[2]);

        let settings = segmented(1, Some(4 * 1024 * 1024));
        let report = JsonReport::new(
            Some(&settings),
            Some(("3662499431".to_string(), "Pack".to_string())),
            Some(&plan_with(Vec::new())),
            TimingsJson::default(),
            None,
            None,
            None,
            None,
            &[row],
            &Ok(()),
        );
        let json = serde_json::to_value(&report).unwrap();
        let uploads = json["uploads"].as_array().expect("the uploads array");
        let sha256 = uploads[0]["sha256"].as_str().expect("the sha256 field");
        assert_eq!(sha256, digest, "{json}");
    }
}
