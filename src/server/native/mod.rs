//! The native SSH/SFTP transport and segmented upload.
//!
//! The segmented writer, atomic publish, checkpoints and resume live here.

pub mod checkpoint;
pub mod connect;
pub mod known_hosts;
pub mod publish;
pub mod schedule;
pub mod secret;
pub mod segment;
pub mod verify;
pub mod writer;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use indicatif::ProgressBar;
use openssh_sftp_client::file::File;
use openssh_sftp_client::Sftp;
use tokio::task::JoinSet;

use self::checkpoint::{Checkpoint, LocalIdentity, Start};
use self::connect::NativeSession;
use self::schedule::Scheduler;
use self::secret::Secret;
use self::segment::{Chunk, SegmentPlan};
use super::progress::{
    clear_bars, range_advanced, range_finished, range_retried, range_started, SegmentedTrace,
};
use crate::error::UksftaError;
use crate::server::config::Settings;
use crate::server::report::UploadUnit;

/// The longest wait between a failed range and its retry.
const MAX_BACKOFF: Duration = Duration::from_secs(8);
/// How often the acknowledged prefix reaches the checkpoint file.
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(1);

/// Upload one file over several connections, verify it and publish it.
/// Returns the verified SHA-256 of the file.
///
/// # Errors
/// Returns `Input` when the plan, a write, the length check, the read-back
/// hash or the publish fails. A failed run keeps the temp file and the
/// checkpoint, so the next run can resume.
pub fn run(
    settings: &Settings,
    state_root: &Path,
    pack_id: &str,
    unit: &UploadUnit,
    trace: Option<SegmentedTrace>,
    password_prompt: bool,
) -> Result<String, UksftaError> {
    let connections = settings.segment.connections.max(1);
    let threads = usize::from(connections.min(4)).max(1);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(threads)
        .enable_all()
        .build()
        .map_err(UksftaError::Io)?;
    runtime.block_on(async {
        transfer(settings, state_root, pack_id, unit, trace, password_prompt).await
    })
}

async fn transfer(
    settings: &Settings,
    state_root: &Path,
    pack_id: &str,
    unit: &UploadUnit,
    trace: Option<SegmentedTrace>,
    password_prompt: bool,
) -> Result<String, UksftaError> {
    let plan = SegmentPlan::new(
        unit.size,
        settings.segment.connections,
        settings.segment.chunk,
    )?;
    let run_id = writer::run_id();
    let aside_probe = writer::plan_aside_name(&unit.remote, &run_id);
    writer::assert_not_loadable(&aside_probe)?;

    let _lock = writer::take_run_lock(state_root, pack_id)?;
    let password = if password_prompt {
        let (user, host) = connect::split_destination(&settings.destination)?;
        // An explicit prompt owns the credential: a failure to read it is
        // fatal, so the run cannot fall back to another credential.
        match secret::prompt(&host, &user) {
            Ok(secret) => Some(Arc::new(secret)),
            Err(UksftaError::Input(message)) => return Err(UksftaError::Auth(message)),
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let coordinator = connect::connect_with(settings, password.as_deref()).await?;
    let client = coordinator.sftp;
    let buffer = settings.transfer.buffer;

    let (start, temp) =
        checkpoint::decide(&client, state_root, pack_id, unit, settings, buffer).await?;
    writer::assert_not_loadable(&temp)?;

    let directory = remote_dir(&unit.remote);
    writer::clean_orphans(&client, directory, Some(&temp)).await?;
    writer::check_space(&client, directory, unit.size.saturating_mul(2)).await?;

    let identity = LocalIdentity::read(&unit.local)?;
    let from = match start {
        Start::Fresh => 0,
        Start::Resume { from } => from,
    };
    let job = Job {
        settings,
        client: &client,
        unit,
        plan: &plan,
        temp: &temp,
        run_id: &run_id,
        from,
        buffer,
        trace,
        state_root,
        pack_id,
        identity: &identity,
        password: password.as_ref(),
    };
    finish_job(&job).await
}

/// Everything one segmented transfer needs.
struct Job<'a> {
    settings: &'a Settings,
    client: &'a Sftp,
    unit: &'a UploadUnit,
    plan: &'a SegmentPlan,
    temp: &'a str,
    run_id: &'a str,
    from: u64,
    buffer: u32,
    trace: Option<SegmentedTrace>,
    state_root: &'a Path,
    pack_id: &'a str,
    identity: &'a LocalIdentity,
    password: Option<&'a Arc<Secret>>,
}

async fn finish_job(job: &Job<'_>) -> Result<String, UksftaError> {
    let size = job.unit.size;
    if job.from < size {
        send_ranges(job).await?;
    }

    // The transfer may have replaced every worker connection, so the
    // completion steps use a fresh one.
    let coordinator = connect::connect_with(job.settings, job.password.map(AsRef::as_ref)).await?;
    let client = &coordinator.sftp;

    writer::finish(client, job.temp, size).await?;

    let hash = verify::hash_remote(client, job.temp, size, job.buffer).await?;
    if hash != job.identity.sha256 {
        return Err(UksftaError::Input(
            "the server received different bytes; nothing was published".to_string(),
        ));
    }

    let mtime = std::fs::metadata(&job.unit.local)
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs());
    publish::publish(client, job.temp, &job.unit.remote, job.run_id, mtime).await?;
    let _ = std::fs::remove_file(checkpoint::path_for(
        job.state_root,
        job.pack_id,
        &job.unit.remote,
    ));
    Ok(hash)
}

/// The acknowledged ranges of one run, and the file that records them.
struct Acks {
    checkpoint: Checkpoint,
    path: std::path::PathBuf,
    done: BTreeSet<u32>,
    last_saved: Instant,
}

impl Acks {
    /// Record one acknowledged range. The first ack is saved at once, so an
    /// early interruption still leaves a usable checkpoint; after that the
    /// file is written at most once a second.
    fn ack(&mut self, index: u32, size: u64) {
        self.done.insert(index);
        checkpoint::recompute_prefix(&mut self.checkpoint, &self.done, size);
        if self.done.len() > 1 && self.last_saved.elapsed() < CHECKPOINT_INTERVAL {
            return;
        }
        self.last_saved = Instant::now();
        if let Err(error) = checkpoint::save(&self.path, &self.checkpoint) {
            eprintln!("Warning: cannot save the checkpoint: {error}");
        }
    }

    fn save_now(&mut self) {
        self.last_saved = Instant::now();
        if let Err(error) = checkpoint::save(&self.path, &self.checkpoint) {
            eprintln!("Warning: cannot save the checkpoint: {error}");
        }
    }
}

/// Send every range at or above `job.from` on the configured connections.
async fn send_ranges(job: &Job<'_>) -> Result<(), UksftaError> {
    if job.from == 0 {
        writer::create_temp(job.client, job.temp).await?;
    } else {
        writer::open_range_writer(job.client, job.temp)
            .await
            .map(drop)?;
    }

    let pending: Vec<Chunk> = job
        .plan
        .chunks
        .iter()
        .copied()
        .filter(|chunk| chunk.start >= job.from)
        .collect();
    let acks = Arc::new(Mutex::new(Acks {
        checkpoint: checkpoint::fresh(&job.unit.remote, job.temp, job.plan.chunk, job.identity),
        path: checkpoint::path_for(job.state_root, job.pack_id, &job.unit.remote),
        done: BTreeSet::from_iter(
            job.plan
                .chunks
                .iter()
                .filter(|chunk| chunk.start + u64::from(chunk.len) <= job.from)
                .map(|chunk| chunk.index),
        ),
        last_saved: Instant::now(),
    }));
    {
        let mut guard = acks
            .lock()
            .map_err(|_| UksftaError::Input("the checkpoint lock is poisoned".to_string()))?;
        guard.checkpoint.acked_prefix = job.from;
        guard.save_now();
    }

    let scheduler = Arc::new(Mutex::new(Scheduler::new(&pending)));
    let bars: Vec<ProgressBar> = job.trace.as_ref().map_or_else(Vec::new, |trace| {
        trace
            .progress
            .connection_bars(&trace.remote, usize::from(job.settings.segment.connections))
    });

    let mut tasks = JoinSet::new();
    for worker in 0..usize::from(job.settings.segment.connections.max(1)) {
        let settings = job.settings.clone();
        let password = job.password.cloned();
        let scheduler = Arc::clone(&scheduler);
        let acks = Arc::clone(&acks);
        let trace = job.trace.clone();
        let bars = bars.clone();
        let temp = job.temp.to_string();
        let local = job.unit.local.clone();
        let buffer = job.buffer;
        let size = job.unit.size;
        tasks.spawn(async move {
            run_worker(
                &settings,
                &local,
                &temp,
                buffer,
                size,
                worker,
                password,
                &scheduler,
                &acks,
                &bars,
                trace.as_ref(),
            )
            .await
        });
    }

    let mut failure: Option<UksftaError> = None;
    while let Some(joined) = tasks.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                failure = Some(error);
                tasks.abort_all();
                break;
            }
            Err(error) => {
                failure = Some(UksftaError::Input(format!(
                    "a segmented worker stopped: {error}"
                )));
                tasks.abort_all();
                break;
            }
        }
    }
    clear_bars(&bars);

    if let Some(error) = failure {
        return Err(error);
    }

    let (finished, remaining, dead, done) = scheduler
        .lock()
        .map_err(|_| UksftaError::Input("the range queue is poisoned".to_string()))
        .map(|guard| {
            (
                guard.is_finished(),
                guard.remaining(),
                guard.dead_workers(),
                guard.done().clone(),
            )
        })?;
    if !finished {
        return Err(UksftaError::Input(format!(
            "no connection could finish the upload: {remaining} range(s) left, {dead} connection(s) gave up"
        )));
    }

    let mut guard = acks
        .lock()
        .map_err(|_| UksftaError::Input("the checkpoint lock is poisoned".to_string()))?;
    checkpoint::recompute_prefix(&mut guard.checkpoint, &done, job.unit.size);
    guard.save_now();
    Ok(())
}

/// One worker: its own connection and handle, its own ranges, its own
/// reconnects.
#[allow(clippy::too_many_arguments)]
async fn run_worker(
    settings: &Settings,
    local: &Path,
    temp: &str,
    buffer: u32,
    size: u64,
    worker: usize,
    password: Option<Arc<Secret>>,
    scheduler: &Arc<Mutex<Scheduler>>,
    acks: &Arc<Mutex<Acks>>,
    bars: &[ProgressBar],
    trace: Option<&SegmentedTrace>,
) -> Result<(), UksftaError> {
    let reconnects = settings.segment.reconnects;
    let mut attempts: u8 = 0;
    let mut session: Option<(NativeSession, File)> = None;

    loop {
        if session.is_none() {
            match open_session(settings, temp, password.as_deref()).await {
                Ok(pair) => session = Some(pair),
                Err(error) if is_fatal(&error) => return Err(error),
                Err(_) => {
                    attempts += 1;
                    if attempts > reconnects {
                        mark_dead(scheduler)?;
                        return Ok(());
                    }
                    tokio::time::sleep(backoff(attempts)).await;
                    continue;
                }
            }
        }

        let next = take(scheduler)?;
        let Some(chunk) = next else {
            break;
        };

        let (current, file) = session
            .as_mut()
            .ok_or_else(|| UksftaError::Input("a segmented worker lost its handle".to_string()))?;
        let mut advance = |bytes: u64| {
            range_advanced(bars, worker, bytes);
            if let Some(trace) = trace {
                trace.progress.advance(&trace.remote, bytes);
            }
        };
        range_started(bars, worker, chunk.index, chunk.len);
        let written = writer::write_range(file, &chunk, local, buffer, &mut advance).await;
        let synced = match written {
            Ok(()) if current.sftp.support_fsync() => file
                .sync_all()
                .await
                .map_err(|error| UksftaError::Input(format!("sftp: {error}"))),
            Ok(()) => Ok(()),
            Err(error) => Err(error),
        };

        match synced {
            Ok(()) => {
                range_finished(bars, worker);
                complete(scheduler, chunk.index)?;
                let mut guard = acks.lock().map_err(|_| {
                    UksftaError::Input("the checkpoint lock is poisoned".to_string())
                })?;
                guard.ack(chunk.index, size);
            }
            Err(error) if is_fatal(&error) => {
                range_retried(bars, worker);
                fail(scheduler, chunk.index)?;
                return Err(error);
            }
            Err(_) => {
                range_retried(bars, worker);
                fail(scheduler, chunk.index)?;
                session = None;
                attempts += 1;
                if attempts > reconnects {
                    mark_dead(scheduler)?;
                    return Ok(());
                }
                tokio::time::sleep(backoff(attempts)).await;
            }
        }
    }

    if let Some((_, file)) = session {
        let _ = file.close().await;
    }
    Ok(())
}

async fn open_session(
    settings: &Settings,
    temp: &str,
    password: Option<&Secret>,
) -> Result<(NativeSession, File), UksftaError> {
    let session = connect::connect_with(settings, password).await?;
    let file = writer::open_range_writer(&session.sftp, temp).await?;
    Ok((session, file))
}

fn take(scheduler: &Arc<Mutex<Scheduler>>) -> Result<Option<Chunk>, UksftaError> {
    scheduler
        .lock()
        .map_err(|_| UksftaError::Input("the range queue is poisoned".to_string()))
        .map(|mut guard| guard.take())
}

fn complete(scheduler: &Arc<Mutex<Scheduler>>, index: u32) -> Result<(), UksftaError> {
    scheduler
        .lock()
        .map_err(|_| UksftaError::Input("the range queue is poisoned".to_string()))
        .map(|mut guard| guard.complete(index))
}

fn fail(scheduler: &Arc<Mutex<Scheduler>>, index: u32) -> Result<(), UksftaError> {
    scheduler
        .lock()
        .map_err(|_| UksftaError::Input("the range queue is poisoned".to_string()))
        .map(|mut guard| guard.fail(index))
}

fn mark_dead(scheduler: &Arc<Mutex<Scheduler>>) -> Result<(), UksftaError> {
    scheduler
        .lock()
        .map_err(|_| UksftaError::Input("the range queue is poisoned".to_string()))
        .map(|mut guard| guard.mark_worker_dead())
}

/// The server extensions the segmented path depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extensions {
    pub posix_rename: bool,
    pub fsync: bool,
}

/// Connect once and report the extensions. Nothing is read or written.
///
/// # Errors
/// Returns `Input` when the connection fails.
pub fn probe(settings: &Settings) -> Result<Extensions, UksftaError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(UksftaError::Io)?;
    runtime.block_on(async {
        let session = connect::connect(settings).await?;
        Ok(Extensions {
            posix_rename: session.sftp.support_posix_rename(),
            fsync: session.sftp.support_fsync(),
        })
    })
}

/// The wait before retry `attempt` (1 based): 1 s, 2 s, 4 s, then 8 s.
fn backoff(attempt: u8) -> Duration {
    let exponent = attempt.saturating_sub(1).min(3);
    Duration::from_secs(1u64 << exponent).min(MAX_BACKOFF)
}

/// A failure a retry cannot fix: the local file, the key, or the host.
pub fn is_fatal(error: &UksftaError) -> bool {
    match error {
        UksftaError::Io(_) => false,
        UksftaError::Parse(_)
        | UksftaError::Check(_)
        | UksftaError::Auth(_)
        | UksftaError::Locked(_) => true,
        UksftaError::Input(message) => {
            const FATAL: [&str; 9] = [
                "cannot read ",
                "cannot seek ",
                "the local file ended early",
                "the server accepted no bytes",
                "refusing a temporary name",
                "known_hosts",
                "host key",
                "authentication",
                "ssh-agent",
            ];
            FATAL.iter().any(|marker| message.contains(marker))
        }
    }
}

/// The directory part of a remote path.
pub fn remote_dir(remote: &str) -> &str {
    match remote.rfind('/') {
        Some(index) if index > 0 => &remote[..index],
        Some(_) => "/",
        None => ".",
    }
}

#[cfg(test)]
mod backoff_tests {
    use super::*;

    #[test]
    fn the_backoff_doubles_and_stops_at_the_cap() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(3), Duration::from_secs(4));
        assert_eq!(backoff(4), Duration::from_secs(8));
        assert_eq!(backoff(9), MAX_BACKOFF);
    }

    #[test]
    fn local_and_identity_failures_are_fatal() {
        for message in [
            "cannot read /local/big.pbo: no such file",
            "cannot seek /local/big.pbo",
            "the local file ended early: /local/big.pbo",
            "the server accepted no bytes",
            "refusing a temporary name that Arma would load: a.pbo",
            "the host key is not in known_hosts",
            "the host key changed",
            "authentication failed",
            "no key file or ssh-agent identity",
        ] {
            assert!(
                is_fatal(&UksftaError::Input(message.to_string())),
                "{message}"
            );
        }
        for message in [
            "sftp: Sftp server reported error kind NoSuchFile",
            "cannot connect to host: connection reset",
            "a segmented worker lost its handle",
        ] {
            assert!(
                !is_fatal(&UksftaError::Input(message.to_string())),
                "{message}"
            );
        }
        assert!(!is_fatal(&UksftaError::Io(std::io::Error::other("pipe"))));
    }

    #[test]
    fn the_remote_directory_is_the_parent() {
        assert_eq!(remote_dir("/srv/a/b.pbo"), "/srv/a");
        assert_eq!(remote_dir("/b.pbo"), "/");
        assert_eq!(remote_dir("b.pbo"), ".");
    }
}
