//! Throughput comparison on the shaped bridge fixture.
//!
//! One 128 MiB upload runs on the system path with `--jobs 1`, then again on
//! the segmented path with four connections. Both run over a
//! `delay 60ms rate 40mbit` link inside the shaped container.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;

use common::{Shaped, PACK_ID, PORT, ROOT};

/// The PBO size for both runs.
const SIZE: u64 = 128 * 1024 * 1024;
/// The pass threshold: the segmented run must be at most two thirds as long.
const MAX_RATIO: f64 = 0.666;

fn live_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Fixture {
    repo: tempfile::TempDir,
    pbo: PathBuf,
    remote_dir: String,
    remote_pbo: String,
    remote_temp: String,
    known_hosts: PathBuf,
    destination: String,
}

impl Fixture {
    fn new(shaped: &Shaped) -> Self {
        let repo = tempfile::TempDir::new().expect("a repository");
        let addons = repo.path().join(".hemttout/release/addons");
        std::fs::create_dir_all(&addons).expect("addons");
        std::fs::write(
            repo.path().join("meta.cpp"),
            format!("protocol = 1;\npublishedid = {PACK_ID};\nname = \"Throughput\";\n"),
        )
        .expect("meta.cpp");
        let pbo = addons.join("throughput.pbo");
        common::write_pattern(&pbo, SIZE);

        let known_hosts = repo.path().join("known_hosts");
        std::fs::write(&known_hosts, shaped.known_hosts_line()).expect("known_hosts");

        let remote_dir = format!("{ROOT}/@{PACK_ID}/addons");
        Self {
            repo,
            pbo,
            remote_pbo: format!("{remote_dir}/throughput.pbo"),
            remote_temp: format!("{remote_dir}/throughput.pbo.uksfta-part"),
            remote_dir,
            known_hosts,
            destination: shaped.destination(),
        }
    }

    fn command(&self, extra: &[&str]) -> Command {
        let mut command = Command::new(cargo_bin("uksfta"));
        command
            .current_dir(self.repo.path())
            .args([
                "server",
                "--upload",
                "--destination",
                &self.destination,
                "--port",
                &PORT.to_string(),
                "--root",
                ROOT,
                "--identity",
                &common::crate_root()
                    .join("tools/sftp-test-server/keys/id_ed25519")
                    .display()
                    .to_string(),
                "--known-hosts",
                &self.known_hosts.display().to_string(),
                "--retries",
                "0",
            ])
            .args(extra);
        command
    }

    /// Remove the remote copy and the local state so each run sends every byte.
    fn reset(&self, shaped: &Shaped) {
        shaped.exec(&["rm", "-rf", &self.remote_pbo, &self.remote_temp]);
        let state = self.repo.path().join(".uksfta");
        let _ = std::fs::remove_dir_all(state);
    }

    fn run(&self, extra: &[&str]) -> (Output, Duration) {
        let started = Instant::now();
        let output = self.command(extra).output().expect("run the upload");
        (output, started.elapsed())
    }
}

fn remote_sha(shaped: &Shaped, path: &str) -> String {
    let out = shaped.exec(&["sha256sum", path]);
    assert!(out.status.success(), "sha256sum failed for {path}");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("a digest")
        .to_string()
}

fn local_sha(path: &Path) -> String {
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("sha256sum");
    assert!(out.status.success(), "cannot hash {}", path.display());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("a digest")
        .to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
#[ignore = "needs the shaped bridge fixture, UKSFTA_SFTP_LIVE=1 and UKSFTA_SFTP_SHAPED=1"]
fn the_segmented_run_beats_one_stream_on_a_shaped_link() {
    if !common::shaped_enabled() {
        return;
    }
    let _serial = live_lock();
    let shaped = match Shaped::start() {
        Ok(shaped) => shaped,
        Err(reason) => {
            eprintln!("skipped: the shaped bridge fixture is unusable: {reason}");
            return;
        }
    };
    shaped.apply();
    let fixture = Fixture::new(&shaped);
    let setup = shaped.exec(&["mkdir", "-p", &fixture.remote_dir]);
    assert!(setup.status.success(), "cannot create the remote directory");
    let expected = local_sha(&fixture.pbo);

    fixture.reset(&shaped);
    let (single_output, single) = fixture.run(&["--jobs", "1"]);
    assert!(
        single_output.status.success(),
        "the single-stream upload failed: {}",
        stderr(&single_output)
    );
    assert_eq!(
        remote_sha(&shaped, &fixture.remote_pbo),
        expected,
        "the single-stream upload is correct"
    );

    fixture.reset(&shaped);
    let (segmented_output, segmented) = fixture.run(&[
        "--segmented",
        "--segment-connections",
        "4",
        "--segment-min",
        "1048576",
        "--segment-chunk",
        "4194304",
    ]);
    assert!(
        segmented_output.status.success(),
        "the segmented upload failed: {}",
        stderr(&segmented_output)
    );
    assert_eq!(
        remote_sha(&shaped, &fixture.remote_pbo),
        expected,
        "the segmented upload is correct"
    );

    let single = single.as_secs_f64();
    let segmented = segmented.as_secs_f64();
    let ratio = segmented / single;
    println!("single={single:.1} segmented={segmented:.1} ratio={ratio:.3}");
    println!("note: wall time includes the segmented read-back verify, which is also shaped");
    assert!(
        ratio <= MAX_RATIO,
        "segmented is not fast enough on the shaped link: single={single:.1}s segmented={segmented:.1}s ratio={ratio:.3} (limit {MAX_RATIO})"
    );
    assert!(
        shaped.qdisc_cleared(),
        "the qdisc was cleared after the run"
    );
}
