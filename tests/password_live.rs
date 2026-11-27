//! CLI-level password cases. They need the Docker fixtures and
//! `UKSFTA_SFTP_LIVE=1`, and the correct-password case needs `script`.
#![cfg(unix)]

mod common;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use assert_cmd::cargo::cargo_bin;

const ROOT: &str = "/home/uksfta/upload";
const PACK_ID: &str = "3662499431";
const SIZE: u64 = 2 * 1024 * 1024;

fn live() -> bool {
    if std::env::var("UKSFTA_SFTP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set UKSFTA_SFTP_LIVE=1 and start the fixture");
        return false;
    }
    true
}

fn live_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn script_available() -> bool {
    Command::new("script")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// The password lives only in the compose service's environment block.
fn fixture_password(service: &str) -> String {
    let out = compose(&[
        "exec",
        "-T",
        service,
        "sh",
        "-c",
        "printf %s \"$FIXTURE_PASSWORD\"",
    ]);
    assert!(
        out.status.success(),
        "cannot read the fixture password: {}",
        String::from_utf8_lossy(&out.stderr).trim()
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn compose(args: &[&str]) -> Output {
    Command::new("docker")
        .current_dir(common::crate_root())
        .args(["compose", "-f", "tools/sftp-test-server/docker-compose.yml"])
        .args(args)
        .output()
        .expect("docker compose")
}

fn remote(service: &str, args: &[&str]) -> Output {
    let mut full = vec!["exec", "-T", "--user", "uksfta", service];
    full.extend_from_slice(args);
    compose(&full)
}

fn remote_exists(service: &str, path: &str) -> bool {
    remote(service, &["test", "-e", path]).status.success()
}

fn remote_sha(service: &str, path: &str) -> String {
    let out = remote(service, &["sha256sum", path]);
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
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()
        .expect("a digest")
        .to_string()
}

fn failed_password_count(service: &str) -> usize {
    let out = compose(&["logs", "--no-color", service]);
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.matches("Failed password for uksfta").count()
}

struct Setup {
    repo: tempfile::TempDir,
    pbo: PathBuf,
    remote_dir: String,
    remote_pbo: String,
    known_hosts: PathBuf,
    port: u16,
    service: &'static str,
}

impl Setup {
    fn new(tag: &str, port: u16, service: &'static str) -> Self {
        Self::with_size(tag, port, service, SIZE)
    }

    fn with_size(tag: &str, port: u16, service: &'static str, size: u64) -> Self {
        let repo = tempfile::TempDir::new().expect("a repository");
        let addons = repo.path().join(".hemttout/release/addons");
        std::fs::create_dir_all(&addons).expect("addons");
        std::fs::write(
            repo.path().join("meta.cpp"),
            format!("protocol = 1;\npublishedid = {PACK_ID};\nname = \"Password Test\";\n"),
        )
        .expect("meta.cpp");
        let pbo = addons.join(format!("pw_{tag}.pbo"));
        common::write_pattern(&pbo, size);

        let known_hosts = repo.path().join("known_hosts");
        std::fs::write(
            &known_hosts,
            format!("[127.0.0.1]:{port} {}\n", common::host_key()),
        )
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
            remote_dir,
            known_hosts,
            port,
            service,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(cargo_bin("uksfta"));
        command
            .current_dir(self.repo.path())
            .env_remove("SSH_AUTH_SOCK")
            .env("HOME", self.repo.path())
            .env("UKSFTA_NO_UPDATE_CHECK", "1")
            .args([
                "server",
                "--upload",
                "--segmented",
                "--password-prompt",
                "--destination",
                "uksfta@127.0.0.1",
                "--port",
                &self.port.to_string(),
                "--root",
                ROOT,
                "--known-hosts",
                &self.known_hosts.display().to_string(),
                "--segment-min",
                "1",
                "--segment-chunk",
                "1048576",
                "--segment-connections",
                "2",
                "--retries",
                "0",
            ]);
        command
    }

    fn prepare(&self) {
        let out = remote(self.service, &["mkdir", "-p", &self.remote_dir]);
        assert!(out.status.success(), "cannot create the remote directory");
        let _ = remote(self.service, &["rm", "-f", &self.remote_pbo]);
    }

    fn has_remote(&self) -> bool {
        remote_exists(self.service, &self.remote_pbo)
    }
}

/// Quote one argument for the shell `script` runs.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Run a command under a PTY. The system `sftp` prompts and the native
/// prompt get their own answers, counted by prompt text so neither feeds on
/// its own echo.
fn run_pty(command: &Command, native: &str, system: &str) -> (Option<i32>, String) {
    let mut parts = vec![shell_quote(&command.get_program().to_string_lossy())];
    for arg in command.get_args() {
        parts.push(shell_quote(&arg.to_string_lossy()));
    }
    let script_line = parts.join(" ");
    let home = command
        .get_envs()
        .find(|(key, _)| *key == "HOME")
        .and_then(|(_, value)| value.map(|value| value.to_os_string()));

    let mut script = Command::new("script");
    script
        .args(["-qec", &script_line, "/dev/null"])
        .current_dir(command.get_current_dir().expect("a working directory"))
        .env_remove("SSH_AUTH_SOCK")
        .env("UKSFTA_NO_UPDATE_CHECK", "1");
    if let Some(home) = home {
        script.env("HOME", home);
    }
    let mut child = script
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("script");
    let mut input = child.stdin.take().expect("stdin");
    let mut output = child.stdout.take().expect("stdout");

    let mut text = String::new();
    let mut system_sent = 0usize;
    let mut native_sent = 0usize;
    let mut buffer = [0u8; 4096];
    loop {
        let read = match output.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let chunk = String::from_utf8_lossy(&buffer[..read]).into_owned();
        text.push_str(&chunk);

        let system_prompts = text.to_ascii_lowercase().matches("password:").count();
        while system_sent < system_prompts {
            std::thread::sleep(Duration::from_millis(150));
            let _ = input.write_all(format!("{system}\n").as_bytes());
            let _ = input.flush();
            system_sent += 1;
        }
        let native_prompts = text.matches("Password for ").count();
        while native_sent < native_prompts {
            std::thread::sleep(Duration::from_millis(150));
            let _ = input.write_all(format!("{native}\n").as_bytes());
            let _ = input.flush();
            native_sent += 1;
        }
    }
    let status = child.wait().expect("wait for script");
    (status.code(), text)
}

/// Every file under the state directory, as text.
fn state_text(repo: &Path) -> String {
    let mut text = String::new();
    let mut stack = vec![repo.join(".uksfta")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(bytes) = std::fs::read(&path) {
                text.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
    }
    text
}

#[test]
#[ignore = "needs the Docker password fixture and UKSFTA_SFTP_LIVE=1"]
fn the_correct_password_uploads_over_a_pty() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    if !script_available() {
        eprintln!("skipped: script is not installed");
        return;
    }
    let setup = Setup::new("correct", 2224, "sftp-password");
    setup.prepare();
    let password = fixture_password("sftp-password");

    let (code, output) = run_pty(&setup.command(), &password, &password);
    println!("correct_password: exit={code:?}");
    assert_eq!(code, Some(0), "the run failed: {output}");
    assert_eq!(
        remote_sha(setup.service, &setup.remote_pbo),
        local_sha(&setup.pbo)
    );
    assert!(
        !output.contains(&password),
        "the password reached the terminal output: {output}"
    );
    assert!(
        !state_text(setup.repo.path()).contains(&password),
        "the password reached .uksfta/"
    );
}

#[test]
#[ignore = "needs the Docker password fixture and UKSFTA_SFTP_LIVE=1"]
fn a_password_prompt_probes_once_and_publishes() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    if !script_available() {
        eprintln!("skipped: script is not installed");
        return;
    }
    // The probe threshold is 256 MiB, so this file makes the probe run.
    let setup = Setup::with_size("pwprobe", 2224, "sftp-password", 300 * 1024 * 1024);
    setup.prepare();
    let password = fixture_password("sftp-password");

    let (code, output) = run_pty(&setup.command(), &password, &password);
    println!("password_probe: exit={code:?}");
    assert_eq!(code, Some(0), "the run failed: {output}");
    assert_eq!(
        output.matches("Password for ").count(),
        1,
        "the native password must be asked once: {output}"
    );
    assert!(
        output.contains("published in"),
        "the segmented transfer must publish: {output}"
    );
    // The system probe runs in batch mode, so it cannot authenticate with a
    // password-only server. The documented fallback keeps the segmented path.
    assert!(
        output.contains("the system path could not authenticate; keeping the segmented path"),
        "the fallback must be reported: {output}"
    );
    assert_eq!(
        remote_sha(setup.service, &setup.remote_pbo),
        local_sha(&setup.pbo)
    );
    assert!(
        !output.contains(&password),
        "the password reached the terminal output: {output}"
    );
}

#[test]
#[ignore = "needs the Docker password fixture and UKSFTA_SFTP_LIVE=1"]
fn the_keyboard_interactive_fixture_uploads_over_a_pty() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    if !script_available() {
        eprintln!("skipped: script is not installed");
        return;
    }
    let setup = Setup::new("kbdint", 2225, "sftp-kbdint");
    setup.prepare();
    let password = fixture_password("sftp-kbdint");

    let (code, output) = run_pty(&setup.command(), &password, &password);
    println!("kbdint: exit={code:?}");
    assert_eq!(code, Some(0), "the run failed: {output}");
    assert_eq!(
        remote_sha(setup.service, &setup.remote_pbo),
        local_sha(&setup.pbo)
    );
    assert!(!output.contains(&password), "the password was printed");
}

#[test]
#[ignore = "needs the Docker password fixture and UKSFTA_SFTP_LIVE=1"]
fn a_wrong_password_exits_two_and_publishes_nothing() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    if !script_available() {
        eprintln!("skipped: script is not installed");
        return;
    }
    let setup = Setup::new("wrong", 2224, "sftp-password");
    setup.prepare();
    let password = fixture_password("sftp-password");
    let before = failed_password_count("sftp-password");

    let (code, output) = run_pty(&setup.command(), "not-the-fixture-password", &password);
    println!("wrong_password: exit={code:?} final={}", setup.has_remote());
    assert_eq!(code, Some(2), "the explicit prompt is fatal: {output}");
    assert!(
        output.contains("(password)"),
        "the refusal names the method: {output}"
    );
    assert_eq!(
        failed_password_count("sftp-password").saturating_sub(before),
        1,
        "exactly one attempt"
    );
    assert!(
        !output.contains("not-the-fixture-password"),
        "the secret was printed"
    );
    assert!(
        !setup.has_remote(),
        "nothing is published, and the key is not used instead"
    );
}

#[test]
#[ignore = "needs the Docker password fixture and UKSFTA_SFTP_LIVE=1"]
fn an_empty_password_exits_two_and_publishes_nothing() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    if !script_available() {
        eprintln!("skipped: script is not installed");
        return;
    }
    let setup = Setup::new("empty", 2224, "sftp-password");
    setup.prepare();
    let password = fixture_password("sftp-password");
    let before = failed_password_count("sftp-password");

    let (code, output) = run_pty(&setup.command(), "", &password);
    println!("empty_password: exit={code:?} final={}", setup.has_remote());
    assert_eq!(code, Some(2), "an empty entry is fatal: {output}");
    assert!(
        output.contains("no password entered"),
        "the empty entry is reported: {output}"
    );
    assert_eq!(
        failed_password_count("sftp-password"),
        before,
        "an empty entry makes no attempt"
    );
    assert!(!setup.has_remote(), "nothing is published");
}

#[test]
#[ignore = "needs the Docker password fixture and UKSFTA_SFTP_LIVE=1"]
fn the_password_prompt_refuses_without_a_terminal() {
    if !live() {
        return;
    }
    let _serial = live_lock();
    let setup = Setup::new("notty", 2224, "sftp-password");
    setup.prepare();
    let before = failed_password_count("sftp-password");

    let output = Command::new("setsid")
        .arg("-w")
        .arg(cargo_bin("uksfta"))
        .args(setup.command().get_args())
        .current_dir(setup.repo.path())
        .env_remove("SSH_AUTH_SOCK")
        .env("UKSFTA_NO_UPDATE_CHECK", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("setsid");
    let errors = String::from_utf8_lossy(&output.stderr);
    println!("no_tty: exit={:?}", output.status.code());
    assert_eq!(output.status.code(), Some(2));
    for name in ["--identity", "ssh-agent", "sftp"] {
        assert!(
            errors.contains(name),
            "the message must name {name}: {errors}"
        );
    }
    assert_eq!(
        failed_password_count("sftp-password"),
        before,
        "no authentication attempt is made"
    );
    assert!(!setup.has_remote(), "nothing is published");
}
