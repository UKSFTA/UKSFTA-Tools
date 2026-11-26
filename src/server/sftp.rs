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
        }
    }

    /// The full argument list for one client run. `-q` suppresses the
    /// client's own meter, so the verify phases stay parseable.
    fn args(&self, quiet: bool) -> Vec<String> {
        let mut args = Vec::new();
        if quiet {
            args.push("-q".to_string());
        }
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
        args.push(self.destination.clone());
        args
    }

    /// A client run with its commands on standard input.
    fn command(&self, quiet: bool) -> Command {
        let mut command = Command::new(&self.binary);
        command.args(self.args(quiet)).stdin(Stdio::piped());
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

        let mut command = self.command(true);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = self.spawn(command)?;
        self.send(&mut child, lines)?;
        let output = child.wait_with_output().map_err(|e| {
            UksftaError::Input(format!("cannot read the output of {}: {e}", self.binary))
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
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

        let mut command = self.command(false);
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
        }
    }

    #[test]
    fn the_client_arguments_stay_inside_the_buffer_limit() {
        let client = client(Transfer::default());

        let args = client.args(true);
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
    fn quiet_adds_the_flag_and_the_meter_path_leaves_it_out() {
        let client = client(Transfer::default());

        let quiet = client.args(true);
        assert!(quiet.iter().any(|arg| arg == "-q"), "{quiet:?}");

        let meter = client.args(false);
        assert!(!meter.iter().any(|arg| arg == "-q"), "{meter:?}");
        assert!(meter.windows(2).any(|pair| pair == ["-B", "32768"]));
        for option in LIVENESS {
            assert!(
                meter.windows(2).any(|pair| pair == ["-o", option]),
                "{meter:?}"
            );
        }
    }

    #[test]
    fn the_default_transfer_values_are_the_baseline() {
        let args = client(Transfer::default()).args(true);

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
        .args(true);

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
