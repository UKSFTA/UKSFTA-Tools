//! Thin wrapper around the `sftp` client.
//!
//! Commands travel on standard input. `sftp -b` puts the client in
//! non-interactive mode and suppresses the password prompt, so the prompt
//! and askpass paths stay available here. One run is one phase, so a
//! prompt appears at most once per phase.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use super::config::Settings;
use crate::error::UksftaError;
use crate::util::sanitize_for_terminal;

/// Environment override for the client binary. A test points this at a stub.
const BINARY_ENV: &str = "UKSFTA_SFTP_BIN";
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// The client. It holds no credentials: `sftp` reads the key, the agent or
/// the terminal prompt itself.
pub struct Sftp {
    binary: String,
    destination: String,
    port: u16,
    identity: Option<PathBuf>,
}

impl Sftp {
    pub fn new(settings: &Settings) -> Self {
        let binary = std::env::var(BINARY_ENV).unwrap_or_else(|_| "sftp".to_string());
        Self {
            binary,
            destination: settings.destination.clone(),
            port: settings.port,
            identity: settings.identity.clone(),
        }
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
        let mut body = String::new();
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }

        let mut command = Command::new(&self.binary);
        command.arg("-q").arg("-P").arg(self.port.to_string());
        if let Some(path) = &self.identity {
            command.arg("-i").arg(path);
        }
        command
            .arg("-R")
            .arg("64")
            .arg("-B")
            .arg("262144")
            .arg(&self.destination)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|e| UksftaError::Input(format!("cannot run {}: {e}", self.binary)))?;
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(body.as_bytes()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(UksftaError::Input(format!(
                    "cannot send commands to {}: {e}",
                    self.binary
                )));
            }
            // The handle drops here, so the client sees end of input.
        }
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
