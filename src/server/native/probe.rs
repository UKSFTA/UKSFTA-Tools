//! The short link probe that chooses between the whole-file and segmented
//! paths.
//!
//! The probe moves 4 MiB per case in the pack folder with hidden names that
//! Arma never loads. It never publishes: every file it writes is removed
//! before the real upload starts.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::connect;
use super::segment::Chunk;
use super::verify::{self, READBACK_BUFFER};
use super::writer;
use crate::error::UksftaError;
use crate::server::config::Settings;
use crate::server::report::{put_line, UploadUnit};
use crate::server::sftp::Sftp;

/// The smallest file worth probing.
pub const MIN_PROBE_BYTES: u64 = 256 * 1024 * 1024;
/// The bytes each probe case moves.
pub const PROBE_BYTES: u64 = 4 * 1024 * 1024;
/// The three probe names. Fixed, so a stale set from a killed run is
/// overwritten and removed by the next probe.
const NAMES: [&str; 3] = [
    ".uksfta-probe-sys",
    ".uksfta-probe-one",
    ".uksfta-probe-two",
];

/// The measured rates, in millions of bytes per second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    pub system_mbps: f64,
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
    /// Estimate both paths. The segmented estimate uses the faster of the
    /// one and two connection write rates, plus the read-back rate.
    pub fn decide(probe: Probe, size: u64) -> Self {
        let bytes = size as f64;
        let whole_rate = (probe.system_mbps * 1_000_000.0).max(f64::EPSILON);
        let write_rate =
            (probe.native_one_mbps.max(probe.native_two_mbps) * 1_000_000.0).max(f64::EPSILON);
        let read_rate = (probe.read_mbps * 1_000_000.0).max(f64::EPSILON);
        let whole_seconds = bytes / whole_rate;
        let segmented_seconds = bytes / write_rate + bytes / read_rate;
        Self {
            probe,
            whole_seconds,
            segmented_seconds,
            segmented: segmented_seconds <= whole_seconds * 0.9,
        }
    }

    /// The line the user sees once the decision is made.
    pub fn decision_line(&self) -> String {
        if self.segmented {
            format!(
                "probe: segmented faster (whole {:.1} s, segmented {:.1} s); using the segmented path",
                self.whole_seconds, self.segmented_seconds
            )
        } else {
            format!(
                "probe: whole-file faster (whole {:.1} s, segmented {:.1} s); using the whole-file path",
                self.whole_seconds, self.segmented_seconds
            )
        }
    }

    /// The measured rates, one line.
    pub fn rates_line(&self) -> String {
        format!(
            "probe: system {:.2}, native one {:.2}, native two {:.2}, read {:.2} MB/s",
            self.probe.system_mbps,
            self.probe.native_one_mbps,
            self.probe.native_two_mbps,
            self.probe.read_mbps
        )
    }
}

/// Remove the probe files and the local probe once the measurement ends.
struct Guard {
    settings: Settings,
    local: PathBuf,
    names: Vec<String>,
}

impl Guard {
    async fn cleanup(&mut self) {
        if !self.names.is_empty() {
            if let Ok(session) = connect::connect_with(&self.settings, None).await {
                for name in self.names.drain(..) {
                    let _ = session.sftp.fs().remove_file(&name).await;
                }
            }
        }
        let _ = std::fs::remove_file(&self.local);
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        let settings = self.settings.clone();
        let local = self.local.clone();
        let names = std::mem::take(&mut self.names);
        let _ = std::thread::spawn(move || {
            if let Ok(runtime) = tokio::runtime::Runtime::new() {
                runtime.block_on(async {
                    if names.is_empty() {
                        return;
                    }
                    if let Ok(session) = connect::connect_with(&settings, None).await {
                        for name in &names {
                            let _ = session.sftp.fs().remove_file(name).await;
                        }
                    }
                });
            }
            let _ = std::fs::remove_file(&local);
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
) -> Result<Decision, UksftaError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(UksftaError::Io)?;
    runtime.block_on(measure(settings, state_root, pack_id, unit))
}

/// Run the probe and return the decision. The caller keeps or drops the
/// segmented path from `decision.segmented`.
///
/// # Errors
/// Returns `Input` when a probe case cannot run. The caller then warns and
/// keeps the segmented path.
pub async fn measure(
    settings: &Settings,
    state_root: &Path,
    pack_id: &str,
    unit: &UploadUnit,
) -> Result<Decision, UksftaError> {
    // The probe writes into the pack folder, so it holds the run lock. The
    // fixed probe names then cannot collide with a second run.
    let _lock = writer::take_run_lock(state_root, pack_id)?;

    // The probe measures the link, not the configured cap.
    let mut probe_settings = settings.clone();
    probe_settings.transfer.limit = None;

    let dir = super::remote_dir(&unit.remote).to_string();
    let names: Vec<String> = NAMES.iter().map(|name| format!("{dir}/{name}")).collect();
    let local = local_probe()?;
    let mut guard = Guard {
        settings: probe_settings.clone(),
        local: local.clone(),
        names: names.clone(),
    };
    let result = probe_inner(&probe_settings, &local, &names, unit.size).await;
    guard.cleanup().await;
    result
}

async fn probe_inner(
    settings: &Settings,
    local: &Path,
    names: &[String],
    size: u64,
) -> Result<Decision, UksftaError> {
    let system = measure_system(settings, local, &names[0]).await?;
    let (one, session) = measure_native(settings, local, &names[1]).await?;
    let two = measure_native_two(settings, local, &names[2], &session).await?;
    let (read, _) = timed(verify::hash_remote(
        &session.sftp,
        &names[2],
        PROBE_BYTES,
        READBACK_BUFFER,
    ))
    .await?;
    let probe = Probe {
        system_mbps: rate(PROBE_BYTES, system),
        native_one_mbps: rate(PROBE_BYTES, one),
        native_two_mbps: rate(PROBE_BYTES, two),
        read_mbps: rate(PROBE_BYTES, read),
    };
    Ok(Decision::decide(probe, size))
}

async fn measure_system(
    settings: &Settings,
    local: &Path,
    remote: &str,
) -> Result<Duration, UksftaError> {
    let unit = UploadUnit {
        local: local.to_path_buf(),
        remote: remote.to_string(),
        size: PROBE_BYTES,
    };
    let line = put_line(&unit)?;
    let (elapsed, _) = timed(async {
        let client = Sftp::new(settings);
        tokio::task::spawn_blocking(move || client.batch(&[line]))
            .await
            .map_err(|error| UksftaError::Input(format!("the probe task stopped: {error}")))?
    })
    .await?;
    Ok(elapsed)
}

async fn measure_native(
    settings: &Settings,
    local: &Path,
    remote: &str,
) -> Result<(Duration, connect::NativeSession), UksftaError> {
    let session = connect::connect_with(settings, None).await?;
    writer::create_temp(&session.sftp, remote).await?;
    let (elapsed, ()) = timed(async {
        write_probe(
            &session.sftp,
            remote,
            local,
            0,
            PROBE_BYTES as u32,
            settings.transfer.buffer,
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
) -> Result<Duration, UksftaError> {
    writer::create_temp(&session.sftp, remote).await?;
    let second = connect::connect_with(settings, None).await?;
    let half = (PROBE_BYTES / 2) as u32;
    let buffer = settings.transfer.buffer;
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
    let mut file = writer::open_range_writer(sftp, remote).await?;
    let chunk = Chunk {
        index: 0,
        start,
        len,
    };
    let written = writer::write_range(&mut file, &chunk, local, buffer, None, &mut |_| {}).await;
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

/// A 4 MiB local file with a repeatable body, in the temporary directory.
fn local_probe() -> Result<PathBuf, UksftaError> {
    use std::io::Write;

    let path = std::env::temp_dir().join(format!(
        "uksfta-probe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default()
    ));
    let mut file = std::fs::File::create(&path)
        .map_err(|error| UksftaError::Input(format!("cannot create the probe file: {error}")))?;
    let block: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();
    let mut left = PROBE_BYTES;
    while left > 0 {
        let want = usize::try_from(left.min(block.len() as u64)).unwrap_or(block.len());
        file.write_all(&block[..want])
            .map_err(|error| UksftaError::Input(format!("cannot write the probe file: {error}")))?;
        left -= want as u64;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(system: f64, one: f64, two: f64, read: f64) -> Probe {
        Probe {
            system_mbps: system,
            native_one_mbps: one,
            native_two_mbps: two,
            read_mbps: read,
        }
    }

    #[test]
    fn a_fast_single_stream_keeps_the_whole_file_path() {
        // 100 MB/s whole against 60 MB/s write plus a 300 MB/s read-back.
        let decision = Decision::decide(probe(100.0, 60.0, 70.0, 300.0), 977 * 1024 * 1024);
        assert!(!decision.segmented);
        assert!(decision.segmented_seconds > decision.whole_seconds);
        assert!(decision.decision_line().contains("whole-file faster"));
    }

    #[test]
    fn a_slow_single_stream_takes_the_segmented_path() {
        // 20 MB/s whole against 80 MB/s across two connections.
        let decision = Decision::decide(probe(20.0, 40.0, 80.0, 300.0), 977 * 1024 * 1024);
        assert!(decision.segmented);
        assert!(decision.decision_line().contains("segmented faster"));
    }

    #[test]
    fn a_ten_percent_gain_is_needed() {
        // whole 10 s; segmented write 10.8 s plus a negligible read.
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
    fn the_lines_name_the_numbers() {
        let decision = Decision::decide(probe(100.0, 60.0, 70.0, 300.0), 256 * 1024 * 1024);
        let rates = decision.rates_line();
        assert!(rates.contains("system 100.00"), "{rates}");
        assert!(rates.contains("native one 60.00"), "{rates}");
        assert!(rates.contains("native two 70.00"), "{rates}");
        assert!(rates.contains("read 300.00"), "{rates}");
        assert!(decision.decision_line().contains("whole "));
        assert!(decision.decision_line().contains("segmented "));
    }
}
