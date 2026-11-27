//! CLI-level live tests: kill-and-resume, corruption refusal, orphan cleanup
//! and no publish on failure. They are `#[ignore]` and need the Docker
//! fixture plus `UKSFTA_SFTP_LIVE=1`.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use assert_cmd::cargo::cargo_bin;

/// The pack id in the test `meta.cpp`.
const PACK_ID: &str = "3662499431";
/// The size of the test PBO. Large enough that a kill lands mid-transfer.
const SIZE: u64 = 80 * 1024 * 1024;
/// The remote root the fixture serves.
const ROOT: &str = "/home/uksfta/upload";

fn live() -> bool {
    if std::env::var("UKSFTA_SFTP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set UKSFTA_SFTP_LIVE=1 and start the fixture");
        return false;
    }
    true
}

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn client_key() -> PathBuf {
    crate_root().join("tools/sftp-test-server/keys/id_ed25519")
}

fn host_key() -> String {
    std::fs::read_to_string(crate_root().join("tools/sftp-test-server/keys/id_ed25519_host.pub"))
        .expect("the fixture host key")
        .trim()
        .to_string()
}

/// A pack repository with one large PBO and a pinned host key.
struct Setup {
    repo: tempfile::TempDir,
    pbo: PathBuf,
    remote_dir: String,
    remote_pbo: String,
    remote_temp: String,
    known_hosts: PathBuf,
    host: String,
    port: u16,
}

impl Setup {
    fn new(tag: &str) -> Self {
        Self::at(tag, "127.0.0.1", 2222)
    }

    fn at(tag: &str, host: &str, port: u16) -> Self {
        let repo = tempfile::TempDir::new().expect("a repository");
        let release = repo.path().join(".hemttout/release");
        let addons = release.join("addons");
        std::fs::create_dir_all(&addons).expect("addons");
        std::fs::write(
            repo.path().join("meta.cpp"),
            format!("protocol = 1;\npublishedid = {PACK_ID};\nname = \"Live Test\";\n"),
        )
        .expect("meta.cpp");

        let pbo = addons.join(format!("live_{tag}.pbo"));
        write_pattern(&pbo, SIZE);

        let known_hosts = repo.path().join("known_hosts");
        std::fs::write(&known_hosts, format!("[{host}]:{port} {}\n", host_key()))
            .expect("known_hosts");

        let remote_dir = format!("{ROOT}/@{PACK_ID}/addons");
        let name = pbo
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        Self {
            repo,
            pbo,
            remote_pbo: format!("{remote_dir}/{name}"),
            remote_temp: format!("{remote_dir}/{name}.uksfta-part"),
            remote_dir,
            known_hosts,
            host: host.to_string(),
            port,
        }
    }

    fn command(&self, extra: &[&str]) -> Command {
        let mut command = Command::new(cargo_bin("uksfta"));
        command
            .current_dir(self.repo.path())
            .args([
                "server",
                "--upload",
                "--segmented",
                "--destination",
                &format!("uksfta@{}", self.host),
                "--port",
                &self.port.to_string(),
                "--root",
                ROOT,
                "--identity",
                &client_key().display().to_string(),
                "--known-hosts",
                &self.known_hosts.display().to_string(),
                "--segment-min",
                "1048576",
                "--segment-chunk",
                "4194304",
                "--retries",
                "0",
            ])
            .args(extra);
        command
    }

    fn spawn(&self, extra: &[&str]) -> std::process::Child {
        self.command(extra)
            .spawn()
            .expect("spawn the segmented upload")
    }

    fn run(&self, extra: &[&str]) -> Output {
        self.command(extra).output().expect("run the command")
    }
}

/// A file of `size` bytes with a repeatable body.
fn write_pattern(path: &Path, size: u64) {
    use std::io::Write;

    let mut file = std::fs::File::create(path).expect("the local PBO");
    let block: Vec<u8> = (0..4096u32).map(|value| (value % 251) as u8).collect();
    let mut left = size;
    while left > 0 {
        let want = usize::try_from(left.min(block.len() as u64)).expect("block");
        file.write_all(&block[..want]).expect("write the PBO");
        left -= want as u64;
    }
}

/// Run one command in the fixture container.
fn fixture(args: &[&str]) -> Output {
    Command::new("docker")
        .args([
            "compose",
            "-f",
            "tools/sftp-test-server/docker-compose.yml",
            "exec",
            "-T",
            "--user",
            "uksfta",
            "sftp",
        ])
        .args(args)
        .output()
        .expect("docker compose exec")
}

/// Run one command in the fixture with `input` on its standard input.
fn fixture_input(args: &[&str], input: &[u8]) -> Output {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("docker")
        .args([
            "compose",
            "-f",
            "tools/sftp-test-server/docker-compose.yml",
            "exec",
            "-T",
            "--user",
            "uksfta",
            "sftp",
        ])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("docker compose exec");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input)
        .expect("write the input");
    child
        .wait_with_output()
        .expect("docker compose exec output")
}

fn remote_len(path: &str) -> Option<u64> {
    let out = fixture(&["stat", "-c", "%s", path]);
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn remote_exists(path: &str) -> bool {
    fixture(&["test", "-e", path]).status.success()
}

fn remote_sha256(path: &str) -> String {
    let out = fixture(&["sha256sum", path]);
    assert!(out.status.success(), "sha256sum failed for {path}");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("a digest")
        .to_string()
}

fn local_sha256(path: &Path) -> String {
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

/// Wait until the checkpoint records a non-zero acknowledged prefix.
fn wait_for_ack(setup: &Setup) -> u64 {
    let state = setup.repo.path().join(".uksfta/upload").join(PACK_ID);
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Ok(entries) = std::fs::read_dir(&state) {
            for entry in entries.flatten() {
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                if let Some(value) = acked_prefix(&text) {
                    if value > 0 {
                        return value;
                    }
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "no acknowledged range reached the checkpoint"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn acked_prefix(text: &str) -> Option<u64> {
    let rest = text.split("\"acked_prefix\":").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Wait for the remote temp to reach `at_least`, or for the run to finish.
fn wait_for_temp(setup: &Setup, at_least: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = false;
    while Instant::now() < deadline {
        match remote_len(&setup.remote_temp) {
            Some(len) => {
                seen = true;
                if len >= at_least {
                    return true;
                }
            }
            None if seen => return false,
            None => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// The cases share one remote pack and its lock, so one runs at a time.
fn live_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Create the remote pack directory once.
fn prepare_remote(setup: &Setup) {
    let out = fixture(&["mkdir", "-p", &setup.remote_dir]);
    assert!(out.status.success(), "cannot create the remote directory");
    let _ = fixture(&["rm", "-f", &setup.remote_pbo, &setup.remote_temp]);
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn kill_and_resume_publishes_equal_bytes() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let setup = Setup::new("resume");
    prepare_remote(&setup);

    // One connection makes the acknowledged prefix advance one range at a
    // time, so the kill always lands with a mid-file checkpoint.
    let mut child = setup.spawn(&["--segment-reconnects", "0", "--segment-connections", "1"]);
    let from = wait_for_ack(&setup);
    assert!(
        from > 0 && from < SIZE,
        "the checkpoint must sit mid-file: {from}"
    );
    child.kill().expect("kill the first run");
    child.wait().expect("reap the first run");

    let output = setup.run(&["--segment-reconnects", "2"]);
    let errors = stderr(&output);
    assert!(output.status.success(), "the resume failed: {errors}");
    assert_eq!(remote_sha256(&setup.remote_pbo), local_sha256(&setup.pbo));
    assert!(
        !errors.contains("restarting"),
        "a valid checkpoint must resume, not restart: {errors}"
    );
    println!(
        "kill_and_resume: local {} remote {}",
        local_sha256(&setup.pbo),
        remote_sha256(&setup.remote_pbo)
    );
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn a_corrupted_prefix_forces_a_restart() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let setup = Setup::new("corrupt");
    prepare_remote(&setup);

    // One connection makes the acknowledged prefix advance one range at a
    // time, so the kill always lands with a mid-file checkpoint.
    let mut child = setup.spawn(&["--segment-reconnects", "0", "--segment-connections", "1"]);
    let from = wait_for_ack(&setup);
    assert!(
        from > 0 && from < SIZE,
        "the checkpoint must sit mid-file: {from}"
    );
    child.kill().expect("kill the first run");
    child.wait().expect("reap the first run");

    // Overwrite one byte inside the acknowledged prefix.
    let out = fixture_input(
        &[
            "dd",
            &format!("of={}", setup.remote_temp),
            "bs=1",
            "seek=0",
            "conv=notrunc",
            "count=1",
        ],
        &[0xff],
    );
    assert!(out.status.success(), "cannot corrupt the prefix");

    let output = setup.run(&["--segment-reconnects", "2"]);
    let errors = stderr(&output);
    assert!(output.status.success(), "the restart failed: {errors}");
    assert!(
        errors.contains("restarting"),
        "the prefix mismatch must be logged: {errors}"
    );
    assert_eq!(remote_sha256(&setup.remote_pbo), local_sha256(&setup.pbo));
    println!(
        "corruption_restart: local {} remote {}",
        local_sha256(&setup.pbo),
        remote_sha256(&setup.remote_pbo)
    );
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn orphan_cleanup_removes_only_old_temps() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let setup = Setup::new("orphan");
    prepare_remote(&setup);

    let old = format!("{}/old.pbo.uksfta-part-old", setup.remote_dir);
    let fresh = format!("{}/new.pbo.uksfta-part-new", setup.remote_dir);
    let seed =
        format!("printf old > '{old}'; printf new > '{fresh}'; touch -d '2 days ago' '{old}'");
    let out = fixture(&["sh", "-c", &seed]);
    assert!(out.status.success(), "cannot seed the orphan files");

    let output = setup.run(&["--segment-reconnects", "2"]);
    let errors = stderr(&output);
    assert!(output.status.success(), "the upload failed: {errors}");
    assert!(!remote_exists(&old), "the old orphan must be removed");
    assert!(remote_exists(&fresh), "a fresh temp must survive");
    println!("orphan_cleanup: removed {old}, kept {fresh}");
}

#[test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
fn a_failed_run_never_publishes() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let setup = Setup::new("fail");
    prepare_remote(&setup);

    let mut child = setup.spawn(&["--segment-reconnects", "0"]);
    assert!(
        wait_for_temp(&setup, 4 * 1024 * 1024),
        "the transfer did not start"
    );
    child.kill().expect("kill the run mid-transfer");
    child.wait().expect("reap the run");

    assert!(
        !remote_exists(&setup.remote_pbo),
        "a failed run must not publish the final name"
    );
    assert!(
        remote_exists(&setup.remote_temp),
        "the temp file is kept for the next run"
    );
    println!("no_publish: {} is absent", setup.remote_pbo);
}

// ---- Shaped bridge cases ---------------------------------------------------

fn shaped_prepare(shaped: &common::Shaped, setup: &Setup) {
    let out = shaped.exec(&["mkdir", "-p", &setup.remote_dir]);
    assert!(out.status.success(), "cannot create the remote directory");
    let _ = shaped.exec(&["rm", "-f", &setup.remote_pbo, &setup.remote_temp]);
}

fn shaped_len(shaped: &common::Shaped, path: &str) -> Option<u64> {
    let out = shaped.exec(&["stat", "-c", "%s", path]);
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

fn shaped_exists(shaped: &common::Shaped, path: &str) -> bool {
    shaped.exec(&["test", "-e", path]).status.success()
}

fn shaped_sha(shaped: &common::Shaped, path: &str) -> String {
    let out = shaped.exec(&["sha256sum", path]);
    assert!(out.status.success(), "sha256sum failed for {path}");
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("a digest")
        .to_string()
}

/// Wait for the shaped temp to reach `at_least`.
fn wait_for_shaped_temp(shaped: &common::Shaped, setup: &Setup, at_least: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut seen = false;
    while Instant::now() < deadline {
        match shaped_len(shaped, &setup.remote_temp) {
            Some(len) => {
                seen = true;
                if len >= at_least {
                    return true;
                }
            }
            None if seen => return false,
            None => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Start the shaped fixture, or print the skip reason and return `None`.
fn shaped_fixture() -> Option<common::Shaped> {
    if !common::shaped_enabled() {
        return None;
    }
    match common::Shaped::start() {
        Ok(shaped) => Some(shaped),
        Err(reason) => {
            eprintln!("skipped: the shaped bridge fixture is unusable: {reason}");
            None
        }
    }
}

#[test]
#[ignore = "needs the shaped bridge fixture, UKSFTA_SFTP_LIVE=1 and UKSFTA_SFTP_SHAPED=1"]
fn a_shaped_kill_leaves_no_final_file_and_resumes() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let Some(shaped) = shaped_fixture() else {
        return;
    };
    shaped.apply();
    let setup = Setup::at("shaped_kill", &shaped.ip, common::PORT);
    shaped_prepare(&shaped, &setup);
    let expected = local_sha256(&setup.pbo);

    let mut child = setup.spawn(&["--segment-reconnects", "0", "--segment-connections", "1"]);
    assert!(
        wait_for_shaped_temp(&shaped, &setup, SIZE * 3 / 10),
        "the shaped transfer did not reach 30%"
    );
    let killed = shaped.exec(&["pkill", "-f", "sftp-server"]);
    assert!(
        killed.status.success(),
        "cannot kill the session: {}",
        String::from_utf8_lossy(&killed.stderr).trim()
    );
    let status = child.wait().expect("wait for the killed run");

    println!(
        "shaped_kill: exit={:?} final={} temp={}",
        status.code(),
        shaped_exists(&shaped, &setup.remote_pbo),
        shaped_exists(&shaped, &setup.remote_temp)
    );
    assert_eq!(status.code(), Some(2), "a killed session exits 2");
    assert!(
        !shaped_exists(&shaped, &setup.remote_pbo),
        "a killed run must not publish the final name"
    );
    assert!(
        shaped_exists(&shaped, &setup.remote_temp),
        "the temp file is kept for the resume"
    );

    shaped.clear();
    assert!(
        shaped.qdisc_cleared(),
        "the qdisc was cleared after the case"
    );
    let output = setup.run(&["--segment-reconnects", "2"]);
    let errors = stderr(&output);
    assert!(output.status.success(), "the resume failed: {errors}");
    assert_eq!(shaped_sha(&shaped, &setup.remote_pbo), expected);
}

#[test]
#[ignore = "needs the shaped bridge fixture, UKSFTA_SFTP_LIVE=1 and UKSFTA_SFTP_SHAPED=1"]
fn a_shaped_loss_burst_completes_or_fails_cleanly() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let Some(shaped) = shaped_fixture() else {
        return;
    };
    shaped.apply();
    let setup = Setup::at("shaped_loss", &shaped.ip, common::PORT);
    shaped_prepare(&shaped, &setup);
    let expected = local_sha256(&setup.pbo);

    let mut child = setup.spawn(&["--segment-reconnects", "3", "--segment-connections", "4"]);
    assert!(
        wait_for_shaped_temp(&shaped, &setup, 4 * 1024 * 1024),
        "the shaped transfer did not start"
    );
    shaped.loss(100);
    std::thread::sleep(Duration::from_secs(5));
    shaped.apply();
    let status = child.wait().expect("wait for the loss run");

    let published = shaped_exists(&shaped, &setup.remote_pbo);
    println!("shaped_loss: exit={:?} final={published}", status.code());
    match status.code() {
        Some(0) => assert!(published, "a successful run must publish the file"),
        Some(2) => {}
        other => panic!("unexpected exit: {other:?}"),
    }
    if published {
        assert_eq!(
            shaped_sha(&shaped, &setup.remote_pbo),
            expected,
            "a published file must match the local file"
        );
    }
    shaped.clear();
    assert!(
        shaped.qdisc_cleared(),
        "the qdisc was cleared after the case"
    );
}

#[test]
#[ignore = "needs the shaped bridge fixture, UKSFTA_SFTP_LIVE=1 and UKSFTA_SFTP_SHAPED=1"]
fn three_shaped_kills_then_a_clean_resume() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let Some(shaped) = shaped_fixture() else {
        return;
    };
    shaped.apply();
    let setup = Setup::at("shaped_three", &shaped.ip, common::PORT);
    shaped_prepare(&shaped, &setup);
    let expected = local_sha256(&setup.pbo);

    for percent in [10u64, 40, 70] {
        let mut child = setup.spawn(&["--segment-reconnects", "0", "--segment-connections", "1"]);
        assert!(
            wait_for_shaped_temp(&shaped, &setup, SIZE * percent / 100),
            "the shaped transfer did not reach {percent}%"
        );
        let killed = shaped.exec(&["pkill", "-f", "sftp-server"]);
        assert!(
            killed.status.success(),
            "cannot kill the session at {percent}%"
        );
        let status = child.wait().expect("wait for the killed run");
        assert_eq!(status.code(), Some(2), "a killed run exits 2 at {percent}%");
        assert!(
            !shaped_exists(&shaped, &setup.remote_pbo),
            "nothing is published at {percent}%"
        );
        println!("shaped_three: killed at {percent}%");
    }

    shaped.clear();
    assert!(
        shaped.qdisc_cleared(),
        "the qdisc was cleared after the case"
    );
    let output = setup.run(&["--segment-reconnects", "2"]);
    let errors = stderr(&output);
    assert!(output.status.success(), "the clean resume failed: {errors}");
    assert_eq!(shaped_sha(&shaped, &setup.remote_pbo), expected);

    let listing = shaped.exec(&["ls", "-1", &setup.remote_dir]);
    let text = String::from_utf8_lossy(&listing.stdout);
    assert!(
        !text.contains(".uksfta-part-") && !text.contains(".uksfta-old-"),
        "no temp residue must remain: {text}"
    );
}
