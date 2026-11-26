//! Open the SFTP subsystem with russh, then authenticate with a key file
//! and the SSH agent. There is no password or keyboard-interactive path.

use std::num::NonZeroU16;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use openssh_sftp_client::{Sftp, SftpOptions};
use russh::client::{self, Config, Handle};
use russh::keys::agent::client::AgentClient;
use russh::keys::{HashAlg, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate};
use tokio::net::TcpStream;

use super::known_hosts::{self, Verdict};
use crate::error::UksftaError;
use crate::server::config::Settings;
use crate::server::hostkey::{self, HostKeyPolicy};

/// A connected and authenticated session.
pub struct NativeSession {
    /// Held so the SSH connection outlives the SFTP client built from it.
    #[allow(dead_code)]
    pub handle: Handle<Handler>,
    pub sftp: Sftp,
    /// The accept-new line, when this connection added a key. Wave 3 prints it.
    #[allow(dead_code)]
    pub notice: Option<String>,
}

impl std::fmt::Debug for Handler {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Handler")
            .field("host", &self.host)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

/// The client handler that pins the host key before authentication.
pub struct Handler {
    destination: String,
    known_hosts: PathBuf,
    host: String,
    port: u16,
    policy: HostKeyPolicy,
    rejection: Arc<Mutex<Option<String>>>,
    notice: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = server_key.public_key();
        if let Err(error) = known_hosts::refuse_markers(&self.known_hosts, &self.host, self.port) {
            self.reject(error.to_string());
            return Ok(false);
        }
        match known_hosts::check(&self.known_hosts, &self.host, self.port, &key) {
            Ok(Verdict::Known) => Ok(true),
            Ok(Verdict::Changed { line }) => {
                self.reject(changed_message(
                    &self.destination,
                    &self.known_hosts,
                    line,
                    &key,
                ));
                Ok(false)
            }
            Ok(Verdict::Unknown) if self.policy == HostKeyPolicy::AcceptNew => {
                let before = hostkey::snapshot(&self.known_hosts);
                match known_hosts::append(&self.known_hosts, &self.host, self.port, &key) {
                    Ok(()) => {
                        if let Some(notice) = hostkey::acceptance_notice(
                            before,
                            hostkey::snapshot(&self.known_hosts),
                            &self.destination,
                            &self.known_hosts,
                        ) {
                            self.remember_notice(notice);
                        }
                        Ok(true)
                    }
                    Err(error) => {
                        self.reject(error.to_string());
                        Ok(false)
                    }
                }
            }
            Ok(Verdict::Unknown) => {
                self.reject(unknown_message(&self.destination, &self.known_hosts, &key));
                Ok(false)
            }
            Err(error) => {
                self.reject(error.to_string());
                Ok(false)
            }
        }
    }
}

impl Handler {
    fn reject(&self, message: String) {
        if let Ok(mut slot) = self.rejection.lock() {
            *slot = Some(message);
        }
    }

    fn remember_notice(&self, notice: String) {
        if let Ok(mut slot) = self.notice.lock() {
            *slot = Some(notice);
        }
    }
}

/// Split `user@host`. A bracketed IPv6 host loses its brackets.
///
/// # Errors
/// Returns `Input` when the user or host is empty or there is no `@`.
pub fn split_destination(destination: &str) -> Result<(String, String), UksftaError> {
    let Some((user, host)) = destination.rsplit_once('@') else {
        return Err(UksftaError::Input(format!(
            "destination must be user@host: {destination:?}"
        )));
    };
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    if user.is_empty() || host.is_empty() {
        return Err(UksftaError::Input(format!(
            "destination must be user@host: {destination:?}"
        )));
    }
    Ok((user.to_string(), host.to_string()))
}

/// Connect, pin the host key, authenticate and open the SFTP subsystem.
///
/// # Errors
/// Returns `Input` when the destination, the known_hosts file, the key file
/// or the authentication fails, and names the two supported methods.
pub async fn connect(settings: &Settings) -> Result<NativeSession, UksftaError> {
    let (user, host) = split_destination(&settings.destination)?;
    let rejection = Arc::new(Mutex::new(None));
    let notice = Arc::new(Mutex::new(None));
    let handler = Handler {
        destination: settings.destination.clone(),
        known_hosts: settings.known_hosts.clone(),
        host: host.clone(),
        port: settings.port,
        policy: settings.host_key_policy,
        rejection: rejection.clone(),
        notice: notice.clone(),
    };

    let stream = tokio::time::timeout(
        Duration::from_secs(15),
        TcpStream::connect((host.as_str(), settings.port)),
    )
    .await
    .map_err(|_| {
        UksftaError::Input(format!(
            "timeout connecting to {}:{}",
            settings.destination, settings.port
        ))
    })?
    .map_err(|error| {
        UksftaError::Input(format!(
            "cannot connect to {}:{}: {error}",
            settings.destination, settings.port
        ))
    })?;

    let mut handle = client::connect_stream(Arc::new(client_config()), stream, handler)
        .await
        .map_err(|error| {
            let reason = rejection.lock().ok().and_then(|slot| slot.clone());
            match reason {
                Some(message) => UksftaError::Input(message),
                None => UksftaError::Input(format!(
                    "cannot connect to {}: {error}",
                    settings.destination
                )),
            }
        })?;

    authenticate(&mut handle, &user, settings).await?;

    let channel = handle.channel_open_session().await.map_err(|error| {
        UksftaError::Input(format!(
            "cannot open an sftp channel to {}: {error}",
            settings.destination
        ))
    })?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| UksftaError::Input(format!("cannot start the sftp subsystem: {error}")))?;
    let stream = channel.into_stream();
    let (reader, writer) = tokio::io::split(stream);
    let requests = NonZeroU16::new(settings.transfer.requests).unwrap_or(NonZeroU16::MIN);
    let sftp = Sftp::new(
        writer,
        reader,
        SftpOptions::default().max_pending_requests(requests),
    )
    .await
    .map_err(|error| UksftaError::Input(format!("cannot start the sftp client: {error}")))?;

    Ok(NativeSession {
        handle,
        sftp,
        notice: notice.lock().ok().and_then(|slot| slot.clone()),
    })
}

fn client_config() -> Config {
    Config {
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 3,
        inactivity_timeout: Some(Duration::from_secs(30)),
        nodelay: true,
        ..Config::default()
    }
}

async fn authenticate(
    handle: &mut Handle<Handler>,
    user: &str,
    settings: &Settings,
) -> Result<(), UksftaError> {
    if let Some(identity) = &settings.identity {
        let key = russh::keys::load_secret_key(identity, None).map_err(|error| {
            UksftaError::Input(format!(
                "cannot read the identity {}: {error}",
                identity.display()
            ))
        })?;
        let hash = if key.algorithm().is_rsa() {
            handle
                .best_supported_rsa_hash()
                .await
                .map_err(|error| UksftaError::Input(format!("cannot negotiate RSA: {error}")))?
                .flatten()
        } else {
            None
        };
        let result = handle
            .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .map_err(|error| UksftaError::Input(format!("key authentication failed: {error}")))?;
        if result.success() {
            return Ok(());
        }
    }

    if agent_authenticate(handle, user).await? {
        return Ok(());
    }

    Err(UksftaError::Input(format!(
        "no key file or ssh-agent identity could authenticate to {}\nUse --identity PATH with a key file, or load a key into ssh-agent. The default sftp path can still use a password prompt.",
        settings.destination
    )))
}

#[cfg(unix)]
async fn agent_authenticate(handle: &mut Handle<Handler>, user: &str) -> Result<bool, UksftaError> {
    let Ok(mut agent) = AgentClient::connect_env().await else {
        return Ok(false);
    };
    agent_identities(handle, user, &mut agent).await
}

#[cfg(windows)]
async fn agent_authenticate(handle: &mut Handle<Handler>, user: &str) -> Result<bool, UksftaError> {
    let Ok(mut agent) = AgentClient::connect_pageant().await else {
        return Ok(false);
    };
    agent_identities(handle, user, &mut agent).await
}

#[cfg(not(any(unix, windows)))]
async fn agent_authenticate(
    _handle: &mut Handle<Handler>,
    _user: &str,
) -> Result<bool, UksftaError> {
    Ok(false)
}

async fn agent_identities<S>(
    handle: &mut Handle<Handler>,
    user: &str,
    agent: &mut AgentClient<S>,
) -> Result<bool, UksftaError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    if let Some(binding) = handle.agent_session_binding() {
        let _ = agent.bind_session(binding, false).await;
    }
    let Ok(identities) = agent.request_identities().await else {
        return Ok(false);
    };
    for identity in identities {
        let key: PublicKey = identity.public_key().into_owned();
        let hash = if key.algorithm().is_rsa() {
            handle
                .best_supported_rsa_hash()
                .await
                .map_err(|error| UksftaError::Input(format!("cannot negotiate RSA: {error}")))?
                .flatten()
        } else {
            None
        };
        if let Ok(result) = handle
            .authenticate_publickey_with(user, key, hash, agent)
            .await
        {
            if result.success() {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn algorithm_and_fingerprint(key: &PublicKey) -> String {
    format!("{} {}", key.algorithm(), key.fingerprint(HashAlg::Sha256))
}

fn unknown_message(destination: &str, known_hosts: &std::path::Path, key: &PublicKey) -> String {
    format!(
        "host key check failed for {destination}: {} is not in {}\nAdd the key on a trusted machine, or run once with --host-key-policy accept-new",
        algorithm_and_fingerprint(key),
        known_hosts.display()
    )
}

fn changed_message(
    destination: &str,
    known_hosts: &std::path::Path,
    line: usize,
    key: &PublicKey,
) -> String {
    format!(
        "host key for {destination} changed at {}:{line}\nThe server presented {}\nUpdate the entry on a trusted machine if the change is expected",
        known_hosts.display(),
        algorithm_and_fingerprint(key)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const ED25519: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGqJuzzwAO6m0ShdPvpeYvWTgPJWdXbiG8LUZkXwjOKN";

    fn key() -> PublicKey {
        PublicKey::from_openssh(ED25519).expect("a valid test key")
    }

    #[test]
    fn split_destination_reads_user_and_host() {
        assert_eq!(
            split_destination("user@example.com").unwrap(),
            ("user".to_string(), "example.com".to_string())
        );
        assert_eq!(
            split_destination("user@[2001:db8::1]").unwrap(),
            ("user".to_string(), "2001:db8::1".to_string())
        );
        assert_eq!(
            split_destination("user@name@host").unwrap(),
            ("user@name".to_string(), "host".to_string())
        );
    }

    #[test]
    fn split_destination_refuses_a_missing_part() {
        for value in ["host", "user@", "@host", "user@[]"] {
            let error = split_destination(value).unwrap_err();
            assert!(matches!(error, UksftaError::Input(_)), "{value:?}");
        }
    }

    #[test]
    fn the_unknown_message_names_the_host_file_and_remedy() {
        let text = unknown_message(
            "user@example.com",
            std::path::Path::new("/home/user/.ssh/known_hosts"),
            &key(),
        );
        assert!(text.contains("user@example.com"), "{text}");
        assert!(text.contains("/home/user/.ssh/known_hosts"), "{text}");
        assert!(text.contains("SHA256:"), "{text}");
        assert!(text.contains("--host-key-policy accept-new"), "{text}");
    }

    #[test]
    fn the_changed_message_names_the_line() {
        let text = changed_message(
            "user@example.com",
            std::path::Path::new("/home/user/.ssh/known_hosts"),
            7,
            &key(),
        );
        assert!(text.contains(":7"), "{text}");
        assert!(text.contains("SHA256:"), "{text}");
    }

    #[test]
    fn the_no_method_error_names_both_supported_methods() {
        let error = UksftaError::Input(format!(
            "no key file or ssh-agent identity could authenticate to {}",
            "user@example.com"
        ));
        assert!(error.to_string().contains("key file"));
        assert!(error.to_string().contains("ssh-agent"));
    }

    #[test]
    fn the_config_mirrors_the_liveness_values() {
        let config = client_config();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(15)));
        assert_eq!(config.keepalive_max, 3);
        assert_eq!(config.inactivity_timeout, Some(Duration::from_secs(30)));
    }
}
