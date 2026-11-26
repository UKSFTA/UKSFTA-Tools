//! Thin wrapper around the `sftp` client.
//!
//! Commands travel on standard input. `sftp -b` puts the client in
//! non-interactive mode and suppresses the password prompt, so the prompt
//! and askpass paths stay available here. One run is one phase, so a
//! prompt appears at most once per phase.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use super::config::{Settings, Transfer};
use super::hostkey::{self, HostKeyPolicy};
use crate::error::UksftaError;
use crate::util::sanitize_for_terminal;

/// Environment override for the client binary. A test points this at a stub.
const BINARY_ENV: &str = "UKSFTA_SFTP_BIN";
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// Keep the connection alive, and fail a dead link in seconds.
const LIVENESS: [&str; 3] = [
    "ServerAliveInterval=15",
    "ServerAliveCountMax=3",
    "ConnectTimeout=15",
];

/// The client. It holds no credentials: `sftp` reads the key, the agent or
/// the terminal prompt itself.
pub struct Sftp {
    binary: String,
    destination: String,
    port: u16,
    identity: Option<PathBuf>,
    transfer: Transfer,
    known_hosts: PathBuf,
    policy: HostKeyPolicy,
}

impl Sftp {
    pub fn new(settings: &Settings) -> Self {
        let binary = std::env::var(BINARY_ENV).unwrap_or_else(|_| "sftp".to_string());
        Self {
            binary,
            destination: settings.destination.clone(),
            port: settings.port,
            identity: settings.identity.clone(),
            transfer: settings.transfer,
            known_hosts: settings.known_hosts.clone(),
            policy: settings.host_key_policy,
        }
    }

    /// The full argument list for one client run. `LogLevel=ERROR` keeps
    /// ssh warnings out of a successful run, and still shows the host key
    /// error that a failure needs. It does not suppress the meter.
    fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        args.push("-P".to_string());
        args.push(self.port.to_string());
        if let Some(path) = &self.identity {
            args.push("-i".to_string());
            args.push(path.display().to_string());
        }
        args.extend([
            "-R".to_string(),
            self.transfer.requests.to_string(),
            "-B".to_string(),
            self.transfer.buffer.to_string(),
        ]);
        for option in LIVENESS {
            args.push("-o".to_string());
            args.push(option.to_string());
        }
        args.extend(["-o".to_string(), "LogLevel=ERROR".to_string()]);
        args.extend([
            "-o".to_string(),
            format!("StrictHostKeyChecking={}", self.policy.as_str()),
            "-o".to_string(),
            format!("UserKnownHostsFile={}", self.known_hosts.display()),
            "-o".to_string(),
            "UpdateHostKeys=no".to_string(),
        ]);
        args.push(self.destination.clone());
        args
    }

    /// A client run with its commands on standard input.
    fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(self.args()).stdin(Stdio::piped());
        command
    }

    fn spawn(&self, mut command: Command) -> Result<Child, UksftaError> {
        command
            .spawn()
            .map_err(|e| UksftaError::Input(format!("cannot run {}: {e}", self.binary)))
    }

    /// Write the commands, then close standard input so the client sees the
    /// end of the batch.
    fn send(&self, child: &mut Child, lines: &[String]) -> Result<(), UksftaError> {
        let mut body = String::new();
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(body.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(UksftaError::Input(format!(
                    "cannot send commands to {}: {e}",
                    self.binary
                )));
            }
        }
        Ok(())
    }

    /// Send one phase of commands and return the client's standard output.
    ///
    /// # Errors
    /// Returns `Input` when the client cannot start, cannot take the
    /// commands, or exits non-zero.
    pub fn batch(&self, lines: &[String]) -> Result<String, UksftaError> {
        if lines.is_empty() {
            return Ok(String::new());
        }

        let mut command = self.command();
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = self.spawn(command)?;
        self.send(&mut child, lines)?;
        let output = child.wait_with_output().map_err(|e| {
            UksftaError::Input(format!("cannot read the output of {}: {e}", self.binary))
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if let Some(text) =
                hostkey::describe_failure(&stderr, &self.destination, &self.known_hosts)
            {
                return Err(UksftaError::Input(text));
            }
            let detail = last_line(&stderr)
                .or_else(|| last_line(&stdout))
                .unwrap_or_else(|| "no output".to_string());
            return Err(UksftaError::Input(format!("sftp failed: {detail}")));
        }
        Ok(stdout)
    }

    /// Send one phase with the client's meter attached to this terminal.
    /// The output is not captured, so the meter needs a real terminal.
    ///
    /// # Errors
    /// Returns `Input` when the client cannot start, cannot take the
    /// commands, or exits non-zero.
    pub fn batch_progress(&self, lines: &[String]) -> Result<(), UksftaError> {
        if lines.is_empty() {
            return Ok(());
        }

        let mut command = self.command();
        command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        let mut child = self.spawn(command)?;
        self.send(&mut child, lines)?;
        let status = child
            .wait()
            .map_err(|e| UksftaError::Input(format!("cannot wait for {}: {e}", self.binary)))?;
        if !status.success() {
            let detail = status.code().map_or_else(
                || "terminated by a signal".to_string(),
                |code| format!("exit {code}"),
            );
            return Err(UksftaError::Input(format!("sftp failed: {detail}")));
        }
        Ok(())
    }
}

/// The last non-empty line of client output, stripped of terminal escapes.
fn last_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .map(|line| sanitize_for_terminal(line).trim().to_string())
}

/// Quote one client argument. A quote or control character in a remote path
/// could start a new command, so it is refused instead.
///
/// # Errors
/// Returns `Input` for an empty value or one that a command cannot hold.
pub fn quote(value: &str) -> Result<String, UksftaError> {
    let unsafe_char =
        value.is_empty() || value.contains('"') || value.chars().any(char::is_control);
    if unsafe_char {
        return Err(UksftaError::Input(format!(
            "refusing to send an unsafe path: {value:?}"
        )));
    }
    Ok(format!("\"{value}\""))
}

/// A private scratch directory that removes itself, for downloaded files.
pub struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    pub fn new(prefix: &str) -> Result<Self, UksftaError> {
        let path = unique_path(prefix);
        fs::create_dir_all(&path).map_err(UksftaError::Io)?;
        Ok(Self { path })
    }

    pub fn join(&self, relative: &str) -> PathBuf {
        self.path.join(relative)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_path(prefix: &str) -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}{}-{id}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(transfer: Transfer) -> Sftp {
        Sftp {
            binary: "sftp".to_string(),
            destination: "tester@example.invalid".to_string(),
            port: 2022,
            identity: Some(PathBuf::from("/home/user/.ssh/id_ed25519")),
            transfer,
            known_hosts: PathBuf::from("/home/user/.ssh/known_hosts"),
            policy: HostKeyPolicy::Yes,
        }
    }

    #[test]
    fn the_client_arguments_stay_inside_the_buffer_limit() {
        let client = client(Transfer::default());

        let args = client.args();
        assert!(
            !args.iter().any(|arg| arg == "262144"),
            "the buffer must stay inside the server-safe limit: {args:?}"
        );
        let buffer = args
            .iter()
            .position(|arg| arg == "-B")
            .map(|i| args[i + 1].clone());
        assert_eq!(buffer.as_deref(), Some("32768"));
        for option in LIVENESS {
            assert!(
                args.windows(2).any(|pair| pair == ["-o", option]),
                "{args:?}"
            );
        }
        assert_eq!(
            args.last().map(String::as_str),
            Some("tester@example.invalid")
        );
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-i", "/home/user/.ssh/id_ed25519"]));
    }

    #[test]
    fn the_error_log_level_replaces_quiet_on_every_run() {
        let args = client(Transfer::default()).args();

        assert!(
            !args.iter().any(|arg| arg == "-q"),
            "no run may pass -q: {args:?}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["-o", "LogLevel=ERROR"]),
            "{args:?}"
        );
        let level = args.iter().position(|arg| arg == "LogLevel=ERROR");
        let strict = args
            .iter()
            .position(|arg| arg == "StrictHostKeyChecking=yes");
        assert!(
            level < strict,
            "the log level comes before the host key options: {args:?}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["-B", "32768"]),
            "{args:?}"
        );
        for option in LIVENESS {
            assert!(
                args.windows(2).any(|pair| pair == ["-o", option]),
                "{args:?}"
            );
        }
    }

    #[test]
    fn the_default_transfer_values_are_the_baseline() {
        let args = client(Transfer::default()).args();

        assert!(args.windows(2).any(|pair| pair == ["-R", "64"]), "{args:?}");
        assert!(
            args.windows(2).any(|pair| pair == ["-B", "32768"]),
            "{args:?}"
        );
    }

    #[test]
    fn the_configured_transfer_values_reach_the_arguments() {
        let args = client(Transfer {
            buffer: 261_120,
            requests: 128,
        })
        .args();

        assert!(
            args.windows(2).any(|pair| pair == ["-B", "261120"]),
            "{args:?}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["-R", "128"]),
            "{args:?}"
        );
        let requests = args.iter().position(|arg| arg == "-R");
        let buffer = args.iter().position(|arg| arg == "-B");
        assert!(requests < buffer, "-R must come before -B: {args:?}");
    }

    #[test]
    fn the_host_key_options_are_pinned_for_both_policies() {
        for (policy, expected) in [
            (HostKeyPolicy::Yes, "StrictHostKeyChecking=yes"),
            (HostKeyPolicy::AcceptNew, "StrictHostKeyChecking=accept-new"),
        ] {
            let mut pinned = client(Transfer::default());
            pinned.policy = policy;
            let args = pinned.args();

            assert!(
                args.windows(2).any(|pair| pair == ["-o", expected]),
                "{args:?}"
            );
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["-o", "UserKnownHostsFile=/home/user/.ssh/known_hosts"]),
                "{args:?}"
            );
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["-o", "UpdateHostKeys=no"]),
                "{args:?}"
            );
            assert_eq!(
                args.last().map(String::as_str),
                Some("tester@example.invalid"),
                "the destination stays last: {args:?}"
            );
            assert!(
                !args
                    .iter()
                    .any(|arg| arg.contains("BatchMode") || arg == "StrictHostKeyChecking=no"),
                "{args:?}"
            );
        }
    }

    #[test]
    fn quote_wraps_a_plain_path() {
        assert_eq!(quote("/a b/c").unwrap(), "\"/a b/c\"");
    }

    #[test]
    fn quote_refuses_command_injection() {
        assert!(quote("/a\"\nb").is_err());
        assert!(quote("").is_err());
        assert!(quote("/a\ncd /").is_err());
    }

    #[test]
    fn scratch_dir_removes_itself() {
        let dir = ScratchDir::new("uksfta-test-scratch-").unwrap();
        let path = dir.join("x");
        fs::write(&path, b"x").unwrap();
        assert!(path.exists());
        drop(dir);
        assert!(!path.exists());
    }

    #[test]
    fn last_line_skips_blank_and_keeps_text() {
        assert_eq!(last_line("a\n\nb\n").as_deref(), Some("b"));
        assert_eq!(last_line("   \n"), None);
    }
}
