//! The short link probe that chooses between the whole-file and segmented
//! paths.
//!
//! The probe moves small files in the pack folder with hidden names that
//! Arma never loads. It never publishes: every file it writes is removed
//! before the real upload starts.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::connect;
use super::secret::Secret;
use super::segment::Chunk;
use super::verify::{self, READBACK_BUFFER};
use super::writer;
use crate::error::UksftaError;
use crate::server::config::Settings;
use crate::server::report::{put_line, UploadUnit};
use crate::server::sftp::Sftp;

/// The smallest file worth probing.
pub const MIN_PROBE_BYTES: u64 = 256 * 1024 * 1024;
/// The bytes each native probe case moves.
pub const PROBE_BYTES: u64 = 4 * 1024 * 1024;
/// The bytes each system probe stream moves.
pub const SYSTEM_PROBE_BYTES: u64 = 1024 * 1024;
/// The most system streams the probe starts.
pub const MAX_SYSTEM_JOBS: u8 = 4;

/// The system probe streams, which model the `--jobs` path at up to four.
pub fn system_jobs(jobs: u8) -> u8 {
    jobs.clamp(1, MAX_SYSTEM_JOBS)
}

/// The measured rates, in millions of bytes per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    /// Zero when the system path could not authenticate.
    pub system_mbps: f64,
    /// The number of system streams the rate covers.
    pub system_jobs: u8,
    pub native_one_mbps: f64,
    pub native_two_mbps: f64,
    pub read_mbps: f64,
}

/// The time estimate for one file under each path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    pub probe: Probe,
    pub whole_seconds: f64,
    pub segmented_seconds: f64,
    /// True when the segmented estimate is at least 10 percent better.
    pub segmented: bool,
}

impl Decision {
    /// Estimate both paths. The whole-file estimate uses the aggregate system
    /// rate, and the segmented estimate the faster write rate plus the
    /// read-back rate. A failed system probe keeps the segmented path.
    pub fn decide(probe: Probe, size: u64) -> Self {
        let bytes = size as f64;
        let write_rate =
            (probe.native_one_mbps.max(probe.native_two_mbps) * 1_000_000.0).max(f64::EPSILON);
        let read_rate = (probe.read_mbps * 1_000_000.0).max(f64::EPSILON);
        let segmented_seconds = bytes / write_rate + bytes / read_rate;
        if probe.system_mbps <= 0.0 {
            return Self {
                probe,
                whole_seconds: 0.0,
                segmented_seconds,
                segmented: true,
            };
        }
        let whole_seconds = bytes / (probe.system_mbps * 1_000_000.0);
        Self {
            probe,
            whole_seconds,
            segmented_seconds,
            segmented: segmented_seconds <= whole_seconds * 0.9,
        }
    }

    /// The line the user sees once the decision is made.
    pub fn decision_line(&self) -> String {
        if self.probe.system_mbps <= 0.0 {
            return format!(
                "probe: system path unavailable; segmented estimate {:.1} s; using the segmented path",
                self.segmented_seconds
            );
        }
        let kind = if self.segmented {
            "segmented faster"
        } else {
            "whole-file faster"
        };
        format!(
            "probe: {kind} (whole {:.1} s, segmented {:.1} s, system {:.2} MB/s at {} jobs); using the {} path",
            self.whole_seconds,
            self.segmented_seconds,
            self.probe.system_mbps,
            self.probe.system_jobs,
            if self.segmented { "segmented" } else { "whole-file" }
        )
    }

    /// The measured rates, one line.
    pub fn rates_line(&self) -> String {
        format!(
            "probe: system {:.2} MB/s ({} jobs), native one {:.2}, native two {:.2}, read {:.2}",
            self.probe.system_mbps,
            self.probe.system_jobs,
            self.probe.native_one_mbps,
            self.probe.native_two_mbps,
            self.probe.read_mbps
        )
    }
}

/// Remove the probe files and the local probe once the measurement ends.
struct Guard {
    settings: Settings,
    password: Option<Arc<Secret>>,
    locals: Vec<PathBuf>,
    names: Vec<String>,
    session: Option<connect::NativeSession>,
}

impl Guard {
    async fn cleanup(&mut self) {
        if !self.names.is_empty() {
            let password = self.password.clone();
            let session = match self.session.take() {
                Some(session) => Some(session),
                None => connect::connect_with(&self.settings, password.as_deref())
                    .await
                    .ok(),
            };
            if let Some(session) = session {
                for name in self.names.drain(..) {
                    let _ = session.sftp.fs().remove_file(&name).await;
                }
            }
        }
        for local in self.locals.drain(..) {
            let _ = std::fs::remove_file(local);
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let settings = self.settings.clone();
        let password = self.password.clone();
        let locals = std::mem::take(&mut self.locals);
        let names = std::mem::take(&mut self.names);
        let _ = std::thread::spawn(move || {
            if let Ok(runtime) = tokio::runtime::Runtime::new() {
                runtime.block_on(async {
                    if !names.is_empty() {
                        if let Ok(session) =
                            connect::connect_with(&settings, password.as_deref()).await
                        {
                            for name in &names {
                                let _ = session.sftp.fs().remove_file(name).await;
                            }
                        }
                    }
                });
            }
            for local in &locals {
                let _ = std::fs::remove_file(local);
            }
        })
        .join();
    }
}

/// Run the probe on its own runtime. The upload loop is synchronous.
///
/// # Errors
/// The same errors as [`measure`].
pub fn run_blocking(
    settings: &Settings,
    state_root: &Path,
    pack_id: &str,
    unit: &UploadUnit,
    password: Option<&Arc<Secret>>,
    jobs: u8,
) -> Result<Decision, UksftaError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(UksftaError::Io)?;
    runtime.block_on(measure(settings, state_root, pack_id, unit, password, jobs))
}

/// Run the probe and return the decision. The caller keeps or drops the
/// segmented path from `decision.segmented`.
///
/// # Errors
/// Returns `Auth` when the typed password is refused. Returns `Input` when a
/// native probe case cannot run. A system probe that cannot authenticate is
/// not an error: the decision keeps the segmented path.
pub async fn measure(
    settings: &Settings,
    state_root: &Path,
    pack_id: &str,
    unit: &UploadUnit,
    password: Option<&Arc<Secret>>,
    jobs: u8,
) -> Result<Decision, UksftaError> {
    // The probe writes into the pack folder, so it holds the run lock. The
    // fixed probe names then cannot collide with a second run.
    let _lock = writer::take_run_lock(state_root, pack_id)?;

    // The probe measures the link, not the configured cap.
    let mut probe_settings = settings.clone();
    probe_settings.transfer.limit = None;

    let dir = super::remote_dir(&unit.remote).to_string();
    let jobs = system_jobs(jobs);
    let mut names: Vec<String> = (0..jobs)
        .map(|index| format!("{dir}/.uksfta-probe-sys-{index}"))
        .collect();
    let one = format!("{dir}/.uksfta-probe-one");
    let two = format!("{dir}/.uksfta-probe-two");
    names.push(one.clone());
    names.push(two.clone());

    let local = local_probe();
    let local_sys = local_system_probe();
    let mut guard = Guard {
        settings: probe_settings.clone(),
        password: password.cloned(),
        locals: vec![local.clone(), local_sys.clone()],
        names: names.clone(),
        session: None,
    };
    let result = probe_inner(
        &probe_settings,
        password,
        &local,
        &local_sys,
        &one,
        &two,
        &names[..usize::from(jobs)],
        unit.size,
        jobs,
    )
    .await;
    let (result, session) = match result {
        Ok((decision, session)) => (Ok(decision), Some(session)),
        Err(error) => (Err(error), None),
    };
    // The last probe session removes the probe files, so the cleanup costs
    // no second handshake.
    guard.session = session;
    guard.cleanup().await;
    result
}

/// Measure the link. The system streams, the native write and the second
/// native connection are independent, so they run at the same time. Each
/// measured interval still covers only its own bytes: the system probe moves
/// 1 MiB per stream and the native probe 4 MiB, so the overlap costs a
/// handshake's worth of frames, not a share of the measured bytes.
#[allow(clippy::too_many_arguments)]
async fn probe_inner(
    settings: &Settings,
    password: Option<&Arc<Secret>>,
    local: &Path,
    local_sys: &Path,
    one: &str,
    two: &str,
    sys_names: &[String],
    size: u64,
    jobs: u8,
) -> Result<(Decision, connect::NativeSession), UksftaError> {
    let (system, second, native) = tokio::join!(
        measure_system(settings, local_sys, sys_names),
        connect::connect_with(settings, password.map(AsRef::as_ref)),
        measure_native(settings, password, local, one),
    );
    let system = system.ok();
    let second = second?;
    let (one_time, session) = native?;
    let two_time = measure_native_two(settings, local, two, &session, &second).await?;
    let (read, _) = timed(verify::hash_remote(
        &session.sftp,
        two,
        PROBE_BYTES,
        READBACK_BUFFER,
    ))
    .await?;
    let probe = Probe {
        system_mbps: system.map_or(0.0, |elapsed| {
            rate(SYSTEM_PROBE_BYTES.saturating_mul(u64::from(jobs)), elapsed)
        }),
        system_jobs: jobs,
        native_one_mbps: rate(PROBE_BYTES, one_time),
        native_two_mbps: rate(PROBE_BYTES, two_time),
        read_mbps: rate(PROBE_BYTES, read),
    };
    Ok((Decision::decide(probe, size), session))
}

/// Upload one small file on each system stream at the same time.
async fn measure_system(
    settings: &Settings,
    local: &Path,
    names: &[String],
) -> Result<Duration, UksftaError> {
    let mut handles = Vec::new();
    let started = Instant::now();
    for remote in names {
        let unit = UploadUnit {
            local: local.to_path_buf(),
            remote: remote.clone(),
            size: SYSTEM_PROBE_BYTES,
        };
        let line = put_line(&unit)?;
        let settings = settings.clone();
        handles.push(tokio::task::spawn_blocking(move || {
            Sftp::probe(&settings).batch(&[line])
        }));
    }
    for handle in handles {
        handle
            .await
            .map_err(|error| UksftaError::Input(format!("the probe task stopped: {error}")))??;
    }
    Ok(started.elapsed())
}

async fn measure_native(
    settings: &Settings,
    password: Option<&Arc<Secret>>,
    local: &Path,
    remote: &str,
) -> Result<(Duration, connect::NativeSession), UksftaError> {
    let session = connect::connect_with(settings, password.map(AsRef::as_ref)).await?;
    writer::create_temp(&session.sftp, remote).await?;
    let (elapsed, ()) = timed(async {
        write_probe(
            &session.sftp,
            remote,
            local,
            0,
            PROBE_BYTES as u32,
            settings.transfer.buffer.max(super::NATIVE_BUFFER),
        )
        .await
    })
    .await?;
    Ok((elapsed, session))
}

async fn measure_native_two(
    settings: &Settings,
    local: &Path,
    remote: &str,
    session: &connect::NativeSession,
    second: &connect::NativeSession,
) -> Result<Duration, UksftaError> {
    writer::create_temp(&session.sftp, remote).await?;
    let half = (PROBE_BYTES / 2) as u32;
    let buffer = settings.transfer.buffer.max(super::NATIVE_BUFFER);
    let (elapsed, ()) = timed(async {
        let left = write_probe(&session.sftp, remote, local, 0, half, buffer);
        let right = write_probe(&second.sftp, remote, local, u64::from(half), half, buffer);
        tokio::try_join!(left, right)?;
        Ok(())
    })
    .await?;
    Ok(elapsed)
}

async fn write_probe(
    sftp: &openssh_sftp_client::Sftp,
    remote: &str,
    local: &Path,
    start: u64,
    len: u32,
    buffer: u32,
) -> Result<(), UksftaError> {
    let mut file = writer::RangeWriter::open(sftp, remote, 1).await?;
    let mut readers = writer::LocalReaders::open(local, buffer, 1).await?;
    let chunk = Chunk {
        index: 0,
        start,
        len,
    };
    let written = file
        .write(&chunk, &mut readers, buffer, None, &mut |_| {})
        .await;
    let _ = file.close().await;
    written
}

async fn timed<F, T>(future: F) -> Result<(Duration, T), UksftaError>
where
    F: std::future::Future<Output = Result<T, UksftaError>>,
{
    let started = Instant::now();
    let value = future.await?;
    Ok((started.elapsed(), value))
}

fn rate(bytes: u64, elapsed: Duration) -> f64 {
    bytes as f64 / elapsed.as_secs_f64().max(f64::EPSILON) / 1_000_000.0
}

/// A local file of `size` bytes in the temporary directory.
fn local_file(size: u64) -> Result<PathBuf, UksftaError> {
    use std::io::Write;

    let path = std::env::temp_dir().join(format!(
        "uksfta-probe-{}-{}-{size}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    ));
    let mut file = std::fs::File::create(&path)
        .map_err(|error| UksftaError::Input(format!("cannot create the probe file: {error}")))?;
    let block: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();
    let mut left = size;
    while left > 0 {
        let want = usize::try_from(left.min(block.len() as u64)).unwrap_or(block.len());
        file.write_all(&block[..want])
            .map_err(|error| UksftaError::Input(format!("cannot write the probe file: {error}")))?;
        left -= want as u64;
    }
    Ok(path)
}

fn local_probe() -> PathBuf {
    local_file(PROBE_BYTES).expect("the native probe file")
}

fn local_system_probe() -> PathBuf {
    local_file(SYSTEM_PROBE_BYTES).expect("the system probe file")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(system: f64, one: f64, two: f64, read: f64) -> Probe {
        Probe {
            system_mbps: system,
            system_jobs: 4,
            native_one_mbps: one,
            native_two_mbps: two,
            read_mbps: read,
        }
    }

    #[test]
    fn a_fast_single_stream_keeps_the_whole_file_path() {
        let decision = Decision::decide(probe(100.0, 60.0, 70.0, 300.0), 977 * 1024 * 1024);
        assert!(!decision.segmented);
        assert!(decision.segmented_seconds > decision.whole_seconds);
        assert!(decision.decision_line().contains("whole-file faster"));
    }

    #[test]
    fn a_slow_single_stream_takes_the_segmented_path() {
        let decision = Decision::decide(probe(20.0, 40.0, 80.0, 300.0), 977 * 1024 * 1024);
        assert!(decision.segmented);
        assert!(decision.decision_line().contains("segmented faster"));
    }

    #[test]
    fn a_ten_percent_gain_is_needed() {
        let decision = Decision::decide(probe(100.0, 100.0, 100.0, 10_000.0), 1_000_000_000);
        assert!(!decision.segmented, "{decision:?}");
        let just = Decision::decide(probe(100.0, 120.0, 120.0, 10_000.0), 1_000_000_000);
        assert!(just.segmented, "{just:?}");
    }

    #[test]
    fn the_waiting_rate_uses_the_faster_write_probe() {
        let decision = Decision::decide(probe(50.0, 10.0, 40.0, 400.0), 400_000_000);
        let expected = 400.0 / 40.0 + 400.0 / 400.0;
        assert!((decision.segmented_seconds - expected).abs() < 1e-9);
    }

    #[test]
    fn a_failed_system_probe_keeps_the_segmented_path() {
        let decision = Decision::decide(probe(0.0, 60.0, 70.0, 300.0), 977 * 1024 * 1024);
        assert!(decision.segmented);
        assert_eq!(decision.whole_seconds, 0.0);
        assert!(decision.decision_line().contains("system path unavailable"));
    }

    #[test]
    fn the_system_streams_stop_at_four() {
        assert_eq!(system_jobs(1), 1);
        assert_eq!(system_jobs(4), 4);
        assert_eq!(system_jobs(8), 4);
        assert_eq!(system_jobs(0), 1);
    }

    #[test]
    fn the_lines_name_the_job_count_and_the_numbers() {
        let decision = Decision::decide(probe(100.0, 60.0, 70.0, 300.0), 256 * 1024 * 1024);
        let rates = decision.rates_line();
        assert_eq!(
            rates,
            "probe: system 100.00 MB/s (4 jobs), native one 60.00, native two 70.00, read 300.00"
        );
        let line = decision.decision_line();
        assert!(line.contains("at 4 jobs"), "{line}");
        assert!(line.contains("system 100.00 MB/s"), "{line}");
        assert!(line.contains("whole "), "{line}");
        assert!(line.contains("segmented "), "{line}");
    }
}
