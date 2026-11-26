//! Verify and repair the private modpack on a game server over SFTP.
//!
//! Only `<root>/@<publishedid>/` is read or written. Dependency tooling
//! (`mod_sources.txt`, `mods.lock`, the Workshop cache) is not used here.

mod config;
mod hostkey;
mod parse;
mod plan;
mod report;
mod sftp;
mod verify;

use std::collections::HashSet;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::error::UksftaError;
use crate::lock::{load_lock, LockFile};
use crate::modlist::{parse_mod_sources, ModEntry};
use crate::util::human_size;

use self::config::{Overrides, Settings};
use self::hostkey::HostKeyPolicy;
use self::plan::{is_bikey, is_bisign, is_pbo, Action, LocalFile, LocalPack, Plan};
use self::report::{
    plan_workers, print_plan, print_report, put_line, upload_batch, upload_dirs, upload_units,
    UploadUnit,
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
    /// Uploads to run at the same time. One is a single batch.
    pub jobs: u8,
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
}

/// Verify the pack, then upload or prune when asked.
///
/// # Errors
/// Returns `Check` when the pack fails verification, and `Input` or `Parse`
/// for a connection, usage or parse failure.
pub fn run(opts: ServerOptions) -> Result<(), UksftaError> {
    let settings = config::resolve(
        opts.config.as_deref(),
        &Overrides {
            destination: opts.destination.clone(),
            port: opts.port,
            root: opts.root.clone(),
            identity: opts.identity.clone(),
            buffer: opts.buffer,
            requests: opts.requests,
            known_hosts: opts.known_hosts.clone(),
            host_key_policy: opts.host_key_policy.clone(),
        },
    )?;

    hostkey::preflight(&settings)?;

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
    let meta_upload = local
        .meta
        .as_ref()
        .map_or_else(|| local.meta_source.clone(), |file| file.path.clone());
    let scratch = ScratchDir::new("uksfta-server-")?;
    let locked = locked_pbos()?;
    let cross_checks = cross_checks(&local)?;
    let show_progress = match opts.progress {
        Progress::Always => true,
        Progress::Never => false,
        Progress::Auto => opts.jobs == 1 && std::io::stdout().is_terminal(),
    };
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

    println!("pack {} {}", local.id, local.name);
    println!(
        "server: {}:{} root {}",
        settings.destination, settings.port, settings.root
    );
    for warning in &cross_checks {
        eprintln!("Warning: {warning}");
    }

    let mut plan = verifier.verify()?;
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
    print_report(&plan);
    print_plan(&plan);

    if opts.dry_run {
        println!("dry run: no changes made");
    } else if opts.upload {
        plan = upload_with_retries(&verifier, plan, opts.retries, opts.jobs, show_progress)?;
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
                println!("removed {remote}");
            }
        }
    }

    if plan.failed() {
        let mut reasons = Vec::new();
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
        return Err(UksftaError::Check(reasons.join(", ")));
    }
    Ok(())
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

/// Send the upload batch, re-verify, and send again only what is still
/// missing or stale. The wait grows with the attempt number.
fn upload_with_retries(
    verifier: &Verifier<'_>,
    mut plan: Plan,
    retries: u8,
    jobs: u8,
    progress: bool,
) -> Result<Plan, UksftaError> {
    let limit = u16::from(retries) + 1;
    let mut attempted = false;
    for attempt in 1..=limit {
        let batch = upload_batch(&plan)?;
        if batch.is_empty() {
            break;
        }
        attempted = true;
        let files = plan.uploads().count();
        let bytes = plan.upload_bytes();
        let started = Instant::now();
        if jobs <= 1 {
            let sent = if progress {
                verifier.client.batch_progress(&batch)
            } else {
                verifier.client.batch(&batch).map(|_| ())
            };
            if let Err(e) = sent {
                eprintln!("Warning: upload attempt {attempt}: {e}");
            }
        } else {
            send_parallel(verifier, &plan, jobs, attempt)?;
        }
        plan = verifier.verify()?;
        let elapsed = started.elapsed();
        println!(
            "attempt {attempt}: {files} file(s), {} attempted, {:.1} s, {:.2} MB/s",
            human_size(bytes),
            elapsed.as_secs_f64(),
            transfer_rate(bytes, elapsed)
        );
        match next_attempt(&plan, attempt, limit) {
            NextAttempt::Stop => break,
            NextAttempt::Retry { wait } => std::thread::sleep(wait),
        }
    }
    if attempted {
        println!("after upload:");
        print_report(&plan);
    }
    Ok(plan)
}

/// Make the directories once, then send one batch per worker. Each worker
/// builds its own client, so the workers run at the same time.
fn send_parallel(
    verifier: &Verifier<'_>,
    plan: &Plan,
    jobs: u8,
    attempt: u16,
) -> Result<(), UksftaError> {
    let dirs = upload_dirs(plan)?;
    if !dirs.is_empty() {
        if let Err(e) = verifier.client.batch(&dirs) {
            eprintln!("Warning: upload attempt {attempt}: {e}");
        }
    }

    let workers = plan_workers(upload_units(plan), jobs);
    let results: Vec<Result<(), UksftaError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = workers
            .iter()
            .map(|units| scope.spawn(move || upload_worker(verifier.settings, units)))
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
            eprintln!(
                "Warning: upload attempt {attempt} worker {}: {e}",
                index + 1
            );
        }
    }
    Ok(())
}

/// One worker: a private client and its own `put -p` batch.
fn upload_worker(settings: &Settings, units: &[UploadUnit]) -> Result<(), UksftaError> {
    let mut batch = Vec::new();
    for unit in units {
        batch.push(put_line(unit)?);
    }
    Sftp::new(settings).batch(&batch).map(|_| ())
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
        .pbos
        .iter()
        .map(|pbo| pbo.name.to_ascii_lowercase())
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
    let pbos = collect(&addons, is_pbo);
    if pbos.is_empty() {
        return Err(UksftaError::Input(format!(
            "no PBOs in {}",
            addons.display()
        )));
    }
    let signatures = collect(&addons, is_bisign);
    let keys = collect(&dir.join("keys"), is_bikey);
    let meta_file = local_file(&dir.join("meta.cpp"));
    let mod_cpp = local_file(&dir.join("mod.cpp"));

    Ok(LocalPack {
        id,
        name,
        dir,
        pbos,
        signatures,
        keys,
        meta: meta_file,
        mod_cpp,
        meta_source: meta_path.to_path_buf(),
    })
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

/// The regular files in `dir` whose name passes `keep`, sorted by name.
fn collect(dir: &Path, keep: fn(&str) -> bool) -> Vec<LocalFile> {
    let Ok(items) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<LocalFile> = items
        .flatten()
        .filter_map(|item| {
            let path = item.path();
            let name = path.file_name()?.to_str()?;
            if !keep(name) {
                return None;
            }
            let meta = item.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(LocalFile {
                name: name.to_string(),
                path,
                size: meta.len(),
            })
        })
        .collect();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

fn local_file(path: &Path) -> Option<LocalFile> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(LocalFile {
        name: path.file_name()?.to_str()?.to_string(),
        path: path.to_path_buf(),
        size: meta.len(),
    })
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
        assert_eq!(local.pbos.len(), 1);
        assert_eq!(local.signatures.len(), 1);
        assert_eq!(local.keys.len(), 1);
        assert!(local.meta.is_some());
        assert!(local.mod_cpp.is_some());
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
}
