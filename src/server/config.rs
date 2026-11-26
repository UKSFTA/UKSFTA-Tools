//! Server connection settings. Values come from `.uksfta/server.toml` and
//! from command flags. A flag wins over the file.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::hostkey::{self, HostKeyPolicy};
use crate::error::UksftaError;

/// Port used when neither the file nor a flag sets one.
pub const DEFAULT_PORT: u16 = 22;
/// Config path read when `--config` is not given.
pub const DEFAULT_CONFIG: &str = ".uksfta/server.toml";

/// The smallest transfer buffer worth sending.
pub const MIN_BUFFER: u32 = 4096;
/// The largest write a typical server accepts: 256 KiB minus 1024 bytes.
/// `-B 262144` is a local fatal, "Outbound message too long".
pub const MAX_BUFFER: u32 = 261_120;
/// The smallest outstanding-request count `sftp` accepts.
pub const MIN_REQUESTS: u16 = 1;
/// The largest outstanding-request count this command allows.
pub const MAX_REQUESTS: u16 = 256;

/// The sftp transfer tuning. The in-flight data per connection is about
/// `buffer * requests`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    /// The `-B` value in bytes.
    pub buffer: u32,
    /// The `-R` value: outstanding requests.
    pub requests: u16,
}

impl Default for Transfer {
    fn default() -> Self {
        Self {
            buffer: 32_768,
            requests: 64,
        }
    }
}

impl Transfer {
    /// Check both values against the supported ranges.
    ///
    /// # Errors
    /// Returns `Input` when a value is outside its range.
    pub fn validate(self) -> Result<Self, UksftaError> {
        if !(MIN_BUFFER..=MAX_BUFFER).contains(&self.buffer) {
            return Err(UksftaError::Input(format!(
                "buffer must be {MIN_BUFFER}..={MAX_BUFFER} bytes: {}",
                self.buffer
            )));
        }
        if !(MIN_REQUESTS..=MAX_REQUESTS).contains(&self.requests) {
            return Err(UksftaError::Input(format!(
                "requests must be {MIN_REQUESTS}..={MAX_REQUESTS}: {}",
                self.requests
            )));
        }
        Ok(self)
    }
}

/// The file schema. There is deliberately no credential field: `sftp`
/// handles authentication, so no password ever reaches this program.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub destination: Option<String>,
    pub port: Option<u16>,
    pub root: Option<String>,
    pub identity: Option<PathBuf>,
    pub known_hosts: Option<PathBuf>,
    pub host_key_policy: Option<String>,
}

/// Connection fields a flag can override.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub destination: Option<String>,
    pub port: Option<u16>,
    pub root: Option<String>,
    pub identity: Option<PathBuf>,
    pub buffer: Option<u32>,
    pub requests: Option<u16>,
    pub known_hosts: Option<PathBuf>,
    pub host_key_policy: Option<String>,
}

/// Resolved connection settings with every field present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub destination: String,
    pub port: u16,
    pub root: String,
    pub identity: Option<PathBuf>,
    pub transfer: Transfer,
    pub known_hosts: PathBuf,
    pub host_key_policy: HostKeyPolicy,
}

/// Read the config file. A missing file is `None`, so the defaults apply.
///
/// # Errors
/// Returns `Input` when the file cannot be read and `Parse` when it does
/// not match the schema.
pub fn load(path: &Path) -> Result<Option<ServerConfig>, UksftaError> {
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .map_err(|e| UksftaError::Input(format!("cannot read {}: {e}", path.display())))?;
    let config = toml::from_str(&text)
        .map_err(|e| UksftaError::Parse(format!("failed to parse {}: {e}", path.display())))?;
    Ok(Some(config))
}

/// Merge the config file, the defaults and the flags.
///
/// # Errors
/// Returns `Input` when an explicit config path is missing, when a required
/// field is absent after merging, or when a value would break the batch file.
pub fn resolve(config_path: Option<&Path>, overrides: &Overrides) -> Result<Settings, UksftaError> {
    let path = config_path.map_or_else(|| PathBuf::from(DEFAULT_CONFIG), Path::to_path_buf);
    if config_path.is_some() && !path.exists() {
        return Err(UksftaError::Input(format!(
            "no server config at {}",
            path.display()
        )));
    }
    let config = load(&path)?.unwrap_or_default();

    let destination = overrides
        .destination
        .clone()
        .or(config.destination)
        .ok_or_else(|| {
            UksftaError::Input(
                "destination is required: set it in .uksfta/server.toml or pass --destination"
                    .to_string(),
            )
        })?;
    let root = overrides.root.clone().or(config.root).ok_or_else(|| {
        UksftaError::Input(
            "root is required: set it in .uksfta/server.toml or pass --root".to_string(),
        )
    })?;

    let defaults = Transfer::default();
    let transfer = Transfer {
        buffer: overrides.buffer.unwrap_or(defaults.buffer),
        requests: overrides.requests.unwrap_or(defaults.requests),
    }
    .validate()?;

    let host_key_policy = hostkey::resolve_policy(
        overrides.host_key_policy.as_deref(),
        config.host_key_policy.as_deref(),
    )?;
    let known_hosts = hostkey::resolve_known_hosts(
        overrides.known_hosts.as_deref(),
        config.known_hosts.as_deref(),
    )?;

    Ok(Settings {
        destination: check_destination(&destination)?,
        port: overrides.port.or(config.port).unwrap_or(DEFAULT_PORT),
        root: check_root(&root)?,
        identity: overrides.identity.clone().or(config.identity),
        transfer,
        known_hosts,
        host_key_policy,
    })
}

/// A destination is one argv element, so it must not start with `-` and
/// must not carry whitespace or control characters.
fn check_destination(value: &str) -> Result<String, UksftaError> {
    let clean = !value.is_empty()
        && !value.starts_with('-')
        && !value.chars().any(|c| c.is_control() || c.is_whitespace());
    if !clean {
        return Err(UksftaError::Input(format!(
            "invalid destination: {value:?}"
        )));
    }
    Ok(value.to_string())
}

/// A remote root is quoted inside the batch file. A quote or a control
/// character would inject a new batch command, so both are refused.
fn check_root(value: &str) -> Result<String, UksftaError> {
    let unsafe_char = value.contains('"') || value.chars().any(char::is_control);
    if !value.starts_with('/') || unsafe_char {
        return Err(UksftaError::Input(format!(
            "invalid remote root: {value:?} (use an absolute path)"
        )));
    }
    let trimmed = value.trim_end_matches('/');
    Ok(if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides(destination: &str, root: &str) -> Overrides {
        Overrides {
            destination: Some(destination.to_string()),
            root: Some(root.to_string()),
            ..Overrides::default()
        }
    }

    #[test]
    fn resolve_defaults_the_port_and_keeps_the_root() {
        let settings = resolve(None, &overrides("user@host", "/srv/mod/")).unwrap();
        assert_eq!(settings.port, DEFAULT_PORT);
        assert_eq!(settings.root, "/srv/mod");
        assert_eq!(settings.destination, "user@host");
        assert!(settings.identity.is_none());
    }

    #[test]
    fn resolve_reads_the_config_file_when_present() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("server.toml");
        fs::write(
            &path,
            "destination = \"file@host\"\nport = 2022\nroot = \"/\"\n",
        )
        .unwrap();
        let settings = resolve(Some(&path), &Overrides::default()).unwrap();
        assert_eq!(settings.destination, "file@host");
        assert_eq!(settings.port, 2022);
        assert_eq!(settings.root, "/");
    }

    #[test]
    fn resolve_lets_a_flag_override_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("server.toml");
        fs::write(
            &path,
            "destination = \"file@host\"\nport = 2022\nroot = \"/srv\"\n",
        )
        .unwrap();
        let settings = resolve(Some(&path), &overrides("flag@host", "/other")).unwrap();
        assert_eq!(settings.destination, "flag@host");
        assert_eq!(settings.root, "/other");
        assert_eq!(settings.port, 2022, "the file keeps the port");
    }

    #[test]
    fn resolve_requires_a_destination() {
        let err = resolve(None, &Overrides::default()).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
    }

    #[test]
    fn resolve_errors_on_a_missing_explicit_config() {
        let err = resolve(
            Some(Path::new("/nonexistent/uksfta/server.toml")),
            &overrides("a@b", "/x"),
        )
        .unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
    }

    #[test]
    fn resolve_refuses_a_relative_root() {
        let err = resolve(None, &overrides("a@b", "relative/path")).unwrap_err();
        assert!(matches!(err, UksftaError::Input(_)));
    }

    #[test]
    fn resolve_refuses_a_password_field() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("server.toml");
        fs::write(
            &path,
            "destination = \"a@b\"\nroot = \"/x\"\npassword = \"x\"\n",
        )
        .unwrap();
        assert!(matches!(
            resolve(Some(&path), &Overrides::default()),
            Err(UksftaError::Parse(_))
        ));
    }

    #[test]
    fn transfer_defaults_are_the_baseline() {
        let settings = resolve(None, &overrides("a@b", "/x")).unwrap();
        assert_eq!(settings.transfer, Transfer::default());
        assert_eq!(settings.transfer.buffer, 32_768);
        assert_eq!(settings.transfer.requests, 64);
    }

    #[test]
    fn resolve_applies_the_buffer_and_request_flags() {
        let mut over = overrides("a@b", "/x");
        over.buffer = Some(131_072);
        over.requests = Some(128);

        let settings = resolve(None, &over).unwrap();
        assert_eq!(settings.transfer.buffer, 131_072);
        assert_eq!(settings.transfer.requests, 128);
    }

    #[test]
    fn transfer_validation_accepts_the_range_edges() {
        assert!(Transfer {
            buffer: MIN_BUFFER,
            requests: MIN_REQUESTS
        }
        .validate()
        .is_ok());
        assert!(Transfer {
            buffer: MAX_BUFFER,
            requests: MAX_REQUESTS
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn transfer_validation_rejects_a_bad_buffer() {
        for buffer in [0, MAX_BUFFER + 1, 262_144] {
            let err = Transfer {
                buffer,
                requests: 64,
            }
            .validate()
            .unwrap_err();
            assert!(matches!(err, UksftaError::Input(_)), "{buffer}");
        }
    }

    #[test]
    fn transfer_validation_rejects_a_bad_request_count() {
        for requests in [0, MAX_REQUESTS + 1, 257] {
            let err = Transfer {
                buffer: 32_768,
                requests,
            }
            .validate()
            .unwrap_err();
            assert!(matches!(err, UksftaError::Input(_)), "{requests}");
        }
    }

    #[test]
    fn resolve_reads_the_host_key_settings_from_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("server.toml");
        fs::write(
            &path,
            "destination = \"a@b\"\nroot = \"/x\"\nknown_hosts = \"/tmp/kh\"\nhost_key_policy = \"accept-new\"\n",
        )
        .unwrap();

        let settings = resolve(Some(&path), &Overrides::default()).unwrap();
        assert_eq!(settings.known_hosts, PathBuf::from("/tmp/kh"));
        assert_eq!(settings.host_key_policy, HostKeyPolicy::AcceptNew);
    }
}
