//! Pinned known_hosts checking for the native transport.
//!
//! Plain host names, hashed `|1|` names, comma lists, `[host]:port`, `*`/`?`
//! patterns and `!` negation are matched. `@revoked` and `@cert-authority`
//! lines are refused instead of skipped.
//!
//! The resolved crate API is `known_host_keys_path(host, port, path)`, not
//! `known_host_keys_path(host, port)` as the plan wrote; the path argument is
//! required by russh 0.65.4.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use russh::keys::known_hosts::{check_known_hosts_path, known_host_keys_path};
use russh::keys::Error as KeyError;
use russh::keys::PublicKey;

use crate::error::UksftaError;

/// The result of checking one server key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Known,
    Unknown,
    Changed { line: usize },
}

/// `host` for port 22, `[host]:port` otherwise.
pub fn host_token(host: &str, port: u16) -> String {
    if port == 22 {
        host.to_string()
    } else {
        format!("[{host}]:{port}")
    }
}

/// Compare the server key with the pinned file.
///
/// # Errors
/// Returns `Input` when the file cannot be read or a key line cannot parse.
pub fn check(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<Verdict, UksftaError> {
    match check_known_hosts_path(host, port, key, path) {
        Ok(true) => return Ok(Verdict::Known),
        Ok(false) => {}
        Err(KeyError::KeyChanged { line }) => return Ok(Verdict::Changed { line }),
        Err(error) => return Err(read_error(path, &error.to_string())),
    }

    let token = host_token(host, port).to_ascii_lowercase();
    let mut changed = None;
    for (index, line) in read_lines(path)?.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(pattern) = fields.next() else {
            continue;
        };
        if pattern.starts_with('@') || pattern.contains("|1|") {
            continue;
        }
        if !pattern_matches(pattern, &token) {
            continue;
        }
        let (Some(algorithm), Some(data)) = (fields.next(), fields.next()) else {
            continue;
        };
        let Ok(recorded) = PublicKey::from_openssh(&format!("{algorithm} {data}")) else {
            continue;
        };
        if recorded == *key {
            return Ok(Verdict::Known);
        }
        changed.get_or_insert(index + 1);
    }
    if let Some(line) = changed {
        return Ok(Verdict::Changed { line });
    }

    // A hashed or exact entry for this host exists, but none matches the key.
    let entries = known_host_keys_path(host, port, path)
        .map_err(|error| read_error(path, &error.to_string()))?;
    Ok(match entries.first() {
        Some((line, _)) => Verdict::Changed { line: *line },
        None => Verdict::Unknown,
    })
}

/// Refuse `@revoked` and `@cert-authority` lines that apply to the host.
///
/// A hashed marker pattern cannot be evaluated here (it needs SHA-1), so it
/// is refused rather than skipped.
///
/// # Errors
/// Returns `Input` naming the file and line.
pub fn refuse_markers(path: &Path, host: &str, port: u16) -> Result<(), UksftaError> {
    let token = host_token(host, port).to_ascii_lowercase();
    for (index, line) in read_lines(path)?.iter().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(marker) = fields.next() else {
            continue;
        };
        if !matches!(marker, "@revoked" | "@cert-authority") {
            continue;
        }
        let number = index + 1;
        let Some(patterns) = fields.next() else {
            continue;
        };
        if patterns
            .split(',')
            .any(|pattern| pattern.trim_start_matches('!').starts_with("|1|"))
        {
            return Err(UksftaError::Input(format!(
                "{}:{number}: {marker} uses a hashed host pattern, which cannot be evaluated; use a plain host entry",
                path.display()
            )));
        }
        if !pattern_matches(patterns, &token) {
            continue;
        }
        let reason = if marker == "@revoked" {
            "the host key is revoked"
        } else {
            "@cert-authority is refused; pin the host key with a plain entry"
        };
        return Err(UksftaError::Input(format!(
            "{}:{number}: {reason}",
            path.display()
        )));
    }
    Ok(())
}

/// Record one host key under an exclusive lock, then fsync.
///
/// # Errors
/// Returns `Input` when the key cannot render or the file cannot be written.
pub fn append(path: &Path, host: &str, port: u16, key: &PublicKey) -> Result<(), UksftaError> {
    let token = host_token(host, port);
    let key_text = key
        .to_openssh()
        .map_err(|error| UksftaError::Input(format!("cannot render the host key: {error}")))?;
    let entry = format!("{token} {key_text}\n");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(UksftaError::Io)?;
    }

    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(path)
        .map_err(UksftaError::Io)?;
    file.lock().map_err(UksftaError::Io)?;

    let mut existing = String::new();
    (&file)
        .read_to_string(&mut existing)
        .map_err(UksftaError::Io)?;
    if !existing.lines().any(|line| line.trim() == entry.trim()) {
        let mut writer = &file;
        if !existing.is_empty() && !existing.ends_with('\n') {
            writer.write_all(b"\n").map_err(UksftaError::Io)?;
        }
        writer
            .write_all(entry.as_bytes())
            .map_err(UksftaError::Io)?;
    }
    file.sync_all().map_err(UksftaError::Io)?;
    Ok(())
}

fn read_lines(path: &Path) -> Result<Vec<String>, UksftaError> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().map(str::to_string).collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(read_error(path, &error.to_string())),
    }
}

fn read_error(path: &Path, detail: &str) -> UksftaError {
    UksftaError::Input(format!("cannot read {}: {detail}", path.display()))
}

/// A comma-separated pattern matches when any positive entry matches and no
/// negated entry does. `*` and `?` are globs; matching ignores ASCII case.
fn pattern_matches(pattern: &str, token: &str) -> bool {
    let mut positive = false;
    for entry in pattern.split(',') {
        let (negated, entry) = match entry.strip_prefix('!') {
            Some(rest) => (true, rest),
            None => (false, entry),
        };
        if !glob(&entry.to_ascii_lowercase(), token) {
            continue;
        }
        if negated {
            return false;
        }
        positive = true;
    }
    positive
}

fn glob(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    glob_at(&pattern, &text)
}

fn glob_at(pattern: &[char], text: &[char]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some('*') => {
            glob_at(&pattern[1..], text) || (!text.is_empty() && glob_at(pattern, &text[1..]))
        }
        Some('?') => !text.is_empty() && glob_at(&pattern[1..], &text[1..]),
        Some(character) => text.first() == Some(character) && glob_at(&pattern[1..], &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED25519: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGqJuzzwAO6m0ShdPvpeYvWTgPJWdXbiG8LUZkXwjOKN";
    const RSA: &str = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQDBuinmRp2ZnEyRTwMMcc1Dr/0DZT82FCW09dgCUVDPidmsFTdWsq0HBZxV/ubSranFfePxHQTVS2beJkUYTFOxUsIZa4Q7lST9lH6IJ180ch3AWSC3In98AmOePuZ2kuX4VLAledCeLurlvnrFZuQbqL+jPt+v8xZUrJFVeCKtMYN+HMpzheDb0WzIZfYgpuNzhG0d9ZkFXxlOzHWHTVsXDvLn+vZbG+JhJs2k9rLqPGkaYzQcCav/J5t2vmnBwrNNxB5OgacRzyiyMA2d1AgDLD5bYoPKG8G2Yuc0QeauJuQvLnQmKN1+t7OcytEzINNYWhgMlnJQRLFF0/ZNEtp/";
    const HASHED: &str = "|1|DYkr1aHo4R6jLYWW1lXTVsSwW8Y=|Klc9G7hUdfnBqlnSBfzt1u0zJ1A=";
    const HASHED_PORT: &str = "|1|q+bZnGAYovSyJZN9y3kt6v2uDSE=|+vbM8IeREHowJlvcLy0yB0FVEy4=";

    fn key(text: &str) -> PublicKey {
        PublicKey::from_openssh(text).expect("a valid test key")
    }

    fn written(dir: &Path, text: &str) -> std::path::PathBuf {
        let path = dir.join("known_hosts");
        fs::write(&path, text).expect("write known_hosts");
        path
    }

    #[test]
    fn the_host_token_carries_a_non_default_port() {
        assert_eq!(host_token("example.com", 22), "example.com");
        assert_eq!(host_token("example.com", 2222), "[example.com]:2222");
    }

    #[test]
    fn a_plain_entry_is_known() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = written(dir.path(), &format!("example.com {ED25519} comment\n"));

        assert_eq!(
            check(&path, "example.com", 22, &key(ED25519)).unwrap(),
            Verdict::Known
        );
        assert_eq!(
            check(&path, "example.com", 2222, &key(ED25519)).unwrap(),
            Verdict::Unknown
        );
    }

    #[test]
    fn a_hashed_entry_is_known() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = written(dir.path(), &format!("{HASHED} {ED25519}\n"));

        assert_eq!(
            check(&path, "example.com", 22, &key(ED25519)).unwrap(),
            Verdict::Known
        );
    }

    #[test]
    fn a_hashed_non_default_port_is_known() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = written(dir.path(), &format!("{HASHED_PORT} {ED25519}\n"));

        assert_eq!(
            check(&path, "example.com", 2222, &key(ED25519)).unwrap(),
            Verdict::Known
        );
    }

    #[test]
    fn a_comma_list_matches_either_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = written(
            dir.path(),
            &format!("other.example,example.com {ED25519}\n"),
        );

        assert_eq!(
            check(&path, "example.com", 22, &key(ED25519)).unwrap(),
            Verdict::Known
        );
    }

    #[test]
    fn a_glob_matches_and_negation_refuses() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = written(
            dir.path(),
            &format!("*.example.com,!bad.example.com {ED25519}\n"),
        );

        assert_eq!(
            check(&path, "host.example.com", 22, &key(ED25519)).unwrap(),
            Verdict::Known
        );
        assert_eq!(
            check(&path, "bad.example.com", 22, &key(ED25519)).unwrap(),
            Verdict::Unknown
        );
    }

    #[test]
    fn a_different_key_is_changed_and_names_the_line() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = written(dir.path(), &format!("example.com {RSA}\n"));

        assert_eq!(
            check(&path, "example.com", 22, &key(ED25519)).unwrap(),
            Verdict::Changed { line: 1 }
        );
    }

    #[test]
    fn an_absent_host_is_unknown() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = written(dir.path(), &format!("other.example {ED25519}\n"));

        assert_eq!(
            check(&path, "example.com", 22, &key(ED25519)).unwrap(),
            Verdict::Unknown
        );
    }

    #[test]
    fn a_missing_file_is_unknown() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("known_hosts");

        assert_eq!(
            check(&path, "example.com", 22, &key(ED25519)).unwrap(),
            Verdict::Unknown
        );
    }

    #[test]
    fn a_revoked_or_certificate_authority_line_is_refused() {
        for marker in ["@revoked", "@cert-authority"] {
            let dir = tempfile::TempDir::new().unwrap();
            let path = written(dir.path(), &format!("{marker} example.com {ED25519}\n"));

            let error = refuse_markers(&path, "example.com", 22).unwrap_err();
            assert!(error.to_string().contains("1:"), "{error}");
            assert!(refuse_markers(&path, "other.example", 22).is_ok());
        }
    }

    #[test]
    fn a_hashed_marker_line_is_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = written(dir.path(), &format!("@revoked {HASHED} {ED25519}\n"));

        let error = refuse_markers(&path, "example.com", 22).unwrap_err();
        assert!(error.to_string().contains("hashed"), "{error}");
    }

    #[test]
    fn append_records_a_line_that_then_matches_for_both_ports() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("known_hosts");
        let host_key = key(ED25519);

        append(&path, "example.com", 22, &host_key).unwrap();
        append(&path, "example.com", 22, &host_key).unwrap();
        append(&path, "example.com", 2222, &host_key).unwrap();

        assert_eq!(
            check(&path, "example.com", 22, &host_key).unwrap(),
            Verdict::Known
        );
        assert_eq!(
            check(&path, "example.com", 2222, &host_key).unwrap(),
            Verdict::Known
        );
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2, "no duplicate line: {text}");
        assert!(refuse_markers(&path, "example.com", 22).is_ok());
    }
}
