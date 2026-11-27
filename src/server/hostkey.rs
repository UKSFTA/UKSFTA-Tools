//! Host key pinning for the sftp client. The client itself proves the key.
//! This module resolves the policy, checks the file before a run, and turns
//! a verification failure into a message that names the host and the file.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::config::Settings;
use crate::error::UksftaError;
use crate::util::sanitize_for_terminal;

/// The `StrictHostKeyChecking` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostKeyPolicy {
    /// Refuse a server whose key is absent or changed.
    Yes,
    /// Add an unknown key once, and refuse a changed key.
    AcceptNew,
}

impl HostKeyPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            HostKeyPolicy::Yes => "yes",
            HostKeyPolicy::AcceptNew => "accept-new",
        }
    }

    /// Read a policy name.
    ///
    /// # Errors
    /// Returns `Input` for a value that is not `yes` or `accept-new`.
    pub fn parse(value: &str) -> Result<Self, UksftaError> {
        match value {
            "yes" => Ok(HostKeyPolicy::Yes),
            "accept-new" => Ok(HostKeyPolicy::AcceptNew),
            other => Err(UksftaError::Input(format!(
                "host key policy must be yes or accept-new: {other:?}"
            ))),
        }
    }
}

/// The policy from the flag, then the file, then the strict default.
///
/// # Errors
/// Returns `Input` for an unknown policy name.
pub fn resolve_policy(
    flag: Option<&str>,
    file: Option<&str>,
) -> Result<HostKeyPolicy, UksftaError> {
    flag.or(file)
        .map_or(Ok(HostKeyPolicy::Yes), HostKeyPolicy::parse)
}

/// `$HOME/.ssh/known_hosts`.
///
/// # Errors
/// Returns `Input` when the home directory is unknown.
pub fn default_known_hosts() -> Result<PathBuf, UksftaError> {
    let home = dirs::home_dir().ok_or_else(no_home)?;
    Ok(home.join(".ssh").join("known_hosts"))
}

fn no_home() -> UksftaError {
    UksftaError::Input(
        "cannot find the home directory for ~/.ssh/known_hosts; pass --known-hosts PATH"
            .to_string(),
    )
}

/// The known_hosts file from the flag, then the file, then the default.
///
/// # Errors
/// Returns `Input` for an empty path, a quote or control character, or a
/// relative path.
pub fn resolve_known_hosts(
    flag: Option<&Path>,
    file: Option<&Path>,
) -> Result<PathBuf, UksftaError> {
    flag.or(file)
        .map_or_else(default_known_hosts, check_known_hosts)
}

fn check_known_hosts(value: &Path) -> Result<PathBuf, UksftaError> {
    let text = value
        .to_str()
        .ok_or_else(|| UksftaError::Input(format!("invalid known_hosts path: {value:?}")))?;
    if text.is_empty() || text.contains('"') || text.chars().any(char::is_control) {
        return Err(UksftaError::Input(format!(
            "invalid known_hosts path: {text:?}"
        )));
    }
    let expanded = expand_home(text)?;
    if !expanded.is_absolute() {
        return Err(UksftaError::Input(format!(
            "known_hosts must be an absolute path: {text:?}"
        )));
    }
    Ok(expanded)
}

fn expand_home(text: &str) -> Result<PathBuf, UksftaError> {
    if text == "~" || text.starts_with("~/") {
        let home = dirs::home_dir().ok_or_else(no_home)?;
        let rest = text.strip_prefix("~/").unwrap_or_default();
        return Ok(home.join(rest));
    }
    Ok(PathBuf::from(text))
}

/// Under `yes`, the known_hosts file must exist and be readable before a
/// connection starts. `accept-new` may create it, so it has no preflight.
///
/// # Errors
/// Returns `Input` naming the path and both remedies.
pub fn preflight(settings: &Settings) -> Result<(), UksftaError> {
    if settings.host_key_policy != HostKeyPolicy::Yes {
        return Ok(());
    }
    File::open(&settings.known_hosts).map_err(|e| {
        UksftaError::Input(format!(
            "no readable known_hosts file at {}: {e}\nAdd the server key on a trusted machine, or run once with --host-key-policy accept-new",
            settings.known_hosts.display()
        ))
    })?;
    Ok(())
}

/// Name the host, the file and the remedy when the client reports a host
/// key problem.
pub fn describe_failure(stderr: &str, destination: &str, known_hosts: &Path) -> Option<String> {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|line| is_host_key_line(line))?;
    Some(format!(
        "host key check failed for {destination}: {}\nCheck the server key in {}\nAdd the key on a trusted machine, or run once with --host-key-policy accept-new",
        sanitize_for_terminal(line),
        known_hosts.display()
    ))
}

fn is_host_key_line(line: &str) -> bool {
    line.contains("Host key verification failed")
        || line.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
        || (line.contains("No ") && line.contains(" host key is known"))
}

/// The size and modification time of the known_hosts file.
pub fn snapshot(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// A notice when the file gained content, so the run accepted a new key.
pub fn acceptance_notice(
    before: Option<(u64, SystemTime)>,
    after: Option<(u64, SystemTime)>,
    destination: &str,
    path: &Path,
) -> Option<String> {
    if after.is_some() && after != before {
        Some(format!(
            "host key for {destination} added to {}",
            path.display()
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::config::Transfer;

    fn settings(policy: HostKeyPolicy, known_hosts: PathBuf) -> Settings {
        Settings {
            destination: "tester@example.invalid".to_string(),
            port: 2022,
            root: "/srv".to_string(),
            identity: None,
            transfer: Transfer::default(),
            known_hosts,
            host_key_policy: policy,
            launch_line: None,
            server_cfg: None,
            segment: crate::server::config::Segment::default(),
        }
    }

    #[test]
    fn the_default_policy_is_strict() {
        assert_eq!(resolve_policy(None, None).unwrap(), HostKeyPolicy::Yes);
        assert_eq!(
            resolve_policy(Some("accept-new"), None).unwrap(),
            HostKeyPolicy::AcceptNew
        );
        assert_eq!(
            resolve_policy(None, Some("accept-new")).unwrap(),
            HostKeyPolicy::AcceptNew
        );
        assert_eq!(
            resolve_policy(Some("yes"), Some("accept-new")).unwrap(),
            HostKeyPolicy::Yes,
            "a flag wins over the file"
        );
    }

    #[test]
    fn an_unknown_policy_is_refused() {
        let err = resolve_policy(Some("maybe"), None).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
    }

    #[test]
    fn the_default_known_hosts_sits_under_the_home_directory() {
        let path = default_known_hosts().unwrap();
        assert!(path.ends_with(".ssh/known_hosts"), "{}", path.display());
    }

    #[test]
    fn the_known_hosts_source_order_is_flag_then_file_then_default() {
        let default = default_known_hosts().unwrap();
        assert_eq!(resolve_known_hosts(None, None).unwrap(), default);

        let dir = tempfile::TempDir::new().unwrap();
        let from_file_path = dir.path().join("file-kh");
        let from_flag_path = dir.path().join("flag-kh");

        let from_file = resolve_known_hosts(None, Some(&from_file_path)).unwrap();
        assert_eq!(from_file, from_file_path);

        let from_flag = resolve_known_hosts(Some(&from_flag_path), Some(&from_file_path)).unwrap();
        assert_eq!(from_flag, from_flag_path);
    }

    #[test]
    fn a_leading_home_marker_is_expanded() {
        let home = dirs::home_dir().unwrap();
        let path = resolve_known_hosts(Some(Path::new("~/kh")), None).unwrap();
        assert_eq!(path, home.join("kh"));
    }

    #[test]
    fn a_relative_or_unsafe_known_hosts_path_is_refused() {
        for value in ["kh", "", "/a\"b", "/a\nb", "./kh"] {
            let err = resolve_known_hosts(Some(Path::new(value)), None).unwrap_err();
            assert!(matches!(err, UksftaError::Input(_)), "{value:?}");
        }
    }

    #[test]
    fn strict_policy_needs_a_readable_known_hosts_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("known_hosts");

        let err = preflight(&settings(HostKeyPolicy::Yes, missing.clone())).unwrap_err();
        let text = err.to_string();
        assert!(text.contains(&missing.display().to_string()), "{text}");
        assert!(text.contains("--host-key-policy accept-new"), "{text}");

        std::fs::write(&missing, b"key\n").unwrap();
        assert!(preflight(&settings(HostKeyPolicy::Yes, missing)).is_ok());
    }

    #[test]
    fn accept_new_has_no_preflight_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("known_hosts");
        assert!(preflight(&settings(HostKeyPolicy::AcceptNew, missing)).is_ok());
    }

    #[test]
    fn a_host_key_failure_names_the_host_and_the_file() {
        let path = Path::new("/home/user/.ssh/known_hosts");
        for stderr in [
            "Host key verification failed.\n",
            "@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@@\nREMOTE HOST IDENTIFICATION HAS CHANGED!\n",
            "No ed25519 host key is known for example.invalid and you have requested strict checking.\n",
        ] {
            let text = describe_failure(stderr, "tester@example.invalid", path).unwrap();
            assert!(text.contains("tester@example.invalid"), "{text}");
            assert!(text.contains("/home/user/.ssh/known_hosts"), "{text}");
            assert!(text.contains("--host-key-policy accept-new"), "{text}");
        }
    }

    #[test]
    fn other_stderr_is_not_a_host_key_failure() {
        assert!(
            describe_failure("Permission denied (publickey).", "a@b", Path::new("/k")).is_none()
        );
    }

    #[test]
    fn a_new_known_hosts_entry_produces_a_notice() {
        let path = Path::new("/home/user/.ssh/known_hosts");
        let before = Some((10, SystemTime::UNIX_EPOCH));
        let after = Some((40, SystemTime::UNIX_EPOCH));

        let notice = acceptance_notice(before, after, "tester@example.invalid", path).unwrap();
        assert!(
            notice.contains("host key for tester@example.invalid added to"),
            "{notice}"
        );
        assert!(acceptance_notice(after, after, "a@b", path).is_none());
    }
}
