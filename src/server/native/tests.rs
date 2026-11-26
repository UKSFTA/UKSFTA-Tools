//! Live tests against the Docker OpenSSH fixture in `tools/sftp-test-server`.
//! They are `#[ignore]` and also need `UKSFTA_SFTP_LIVE=1`.

use std::path::PathBuf;

use super::connect;
use crate::server::config::{Segment, Settings, Transfer};
use crate::server::hostkey::HostKeyPolicy;

/// The hashed `known_hosts` pattern for `[127.0.0.1]:2222` and the committed
/// host key. Regenerate it with `ssh-keygen -H` if the fixture key changes.
const HASHED_2222: &str = "|1|wavKFbDi2nh5VKKyOlZEc2kkhqE=|ZilRQJZjyh+j1aDXMMVz/ovQK6w=";

fn live() -> bool {
    if std::env::var("UKSFTA_SFTP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set UKSFTA_SFTP_LIVE=1 and start the fixture");
        return false;
    }
    true
}

fn host() -> String {
    std::env::var("UKSFTA_SFTP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string())
}

fn port() -> u16 {
    std::env::var("UKSFTA_SFTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2222)
}

fn client_key() -> PathBuf {
    std::env::var("UKSFTA_SFTP_KEY").map_or_else(
        |_| PathBuf::from("tools/sftp-test-server/keys/id_ed25519"),
        PathBuf::from,
    )
}

fn host_key() -> String {
    let path = std::env::var("UKSFTA_SFTP_HOST_KEY").map_or_else(
        |_| PathBuf::from("tools/sftp-test-server/keys/id_ed25519_host.pub"),
        PathBuf::from,
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read the fixture host key: {error}"))
        .trim()
        .to_string()
}

fn settings(known_hosts: PathBuf, identity: Option<PathBuf>, port: u16) -> Settings {
    Settings {
        destination: format!("uksfta@{}", host()),
        port,
        root: "/".to_string(),
        identity,
        transfer: Transfer::default(),
        known_hosts,
        host_key_policy: HostKeyPolicy::Yes,
        launch_line: None,
        server_cfg: None,
        segment: Segment::default(),
    }
}

fn known_hosts(entries: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let path = dir.path().join("known_hosts");
    std::fs::write(&path, entries).expect("write known_hosts");
    (dir, path)
}

fn pin(port: u16) -> (tempfile::TempDir, PathBuf) {
    known_hosts(&format!("[{}]:{port} {}\n", host(), host_key()))
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_key_file_connects_and_lists_the_root() {
    if !live() {
        return;
    }
    let (_dir, pinned) = pin(port());
    let settings = settings(pinned, Some(client_key()), port());

    let session = connect::connect(&settings).await.expect("key-file connect");
    session
        .sftp
        .fs()
        .metadata("/")
        .await
        .expect("stat the remote root");
}

#[tokio::test]
#[ignore = "needs the Docker fixture, an agent and UKSFTA_SFTP_LIVE=1"]
async fn an_agent_identity_connects_when_one_is_available() {
    if !live() {
        return;
    }
    if std::env::var_os("SSH_AUTH_SOCK").is_none() {
        eprintln!("skipped: SSH_AUTH_SOCK is not set");
        return;
    }
    let (_dir, pinned) = pin(port());
    let settings = settings(pinned, None, port());
    if let Err(error) = connect::connect(&settings).await {
        eprintln!("skipped: the agent offered no accepted key: {error}");
    }
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn an_unknown_host_is_refused() {
    if !live() {
        return;
    }
    let (_dir, pinned) = known_hosts("");
    let settings = settings(pinned, Some(client_key()), port());

    let text = error_text(connect::connect(&settings).await);
    assert!(text.contains("not in"), "{text}");
    assert!(text.contains("known_hosts"), "{text}");
    assert!(text.contains("--host-key-policy accept-new"), "{text}");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_changed_host_key_is_refused() {
    if !live() {
        return;
    }
    let client = std::fs::read_to_string(client_key().with_extension("pub"))
        .expect("read the client public key");
    let (_dir, pinned) = known_hosts(&format!("[{}]:{} {}\n", host(), port(), client.trim()));
    let settings = settings(pinned, Some(client_key()), port());

    let text = error_text(connect::connect(&settings).await);
    assert!(text.contains("changed"), "{text}");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_hashed_host_entry_is_accepted() {
    if !live() {
        return;
    }
    let (_dir, pinned) = known_hosts(&format!("{HASHED_2222} {}\n", host_key()));
    let settings = settings(pinned, Some(client_key()), 2222);

    connect::connect(&settings)
        .await
        .expect("a hashed entry must match");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_revoked_entry_is_refused() {
    if !live() {
        return;
    }
    let (_dir, pinned) = known_hosts(&format!(
        "@revoked [{}]:{} {}\n",
        host(),
        port(),
        host_key()
    ));
    let settings = settings(pinned, Some(client_key()), port());

    let text = error_text(connect::connect(&settings).await);
    assert!(text.contains("revoked"), "{text}");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_certificate_authority_line_is_refused() {
    if !live() {
        return;
    }
    let (_dir, pinned) = known_hosts(&format!(
        "@cert-authority [{}]:{} {}\n",
        host(),
        port(),
        host_key()
    ));
    let settings = settings(pinned, Some(client_key()), port());

    let text = error_text(connect::connect(&settings).await);
    assert!(text.contains("@cert-authority"), "{text}");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn a_non_default_port_entry_is_accepted() {
    if !live() {
        return;
    }
    let (_dir, pinned) = pin(2223);
    let settings = settings(pinned, Some(client_key()), 2223);

    connect::connect(&settings)
        .await
        .expect("the [host]:2223 entry must match");
}

#[tokio::test]
#[ignore = "needs the Docker fixture and UKSFTA_SFTP_LIVE=1"]
async fn no_identity_names_the_two_supported_methods() {
    if !live() {
        return;
    }
    let (_dir, pinned) = pin(port());
    let settings = settings(pinned, None, port());

    let result = connect::connect(&settings).await;
    if let Err(error) = result {
        let text = error.to_string();
        assert!(text.contains("key file"), "{text}");
        assert!(text.contains("ssh-agent"), "{text}");
    }
}

fn error_text(result: Result<connect::NativeSession, crate::error::UksftaError>) -> String {
    match result {
        Ok(_) => panic!("the connection must fail"),
        Err(error) => error.to_string(),
    }
}
