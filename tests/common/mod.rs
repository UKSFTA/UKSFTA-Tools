#![allow(dead_code)]
//! Helpers for the shaped bridge fixture.
//!
//! The shaped container runs on a private bridge with no published ports.
//! It shapes its own `eth0` inside that bridge; no host interface is ever
//! touched. When the bridge cannot be reached, the shaped tests skip with a
//! printed reason.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::Duration;

/// The pack id in the test `meta.cpp`.
pub const PACK_ID: &str = "3662499431";
/// The remote root the fixture serves.
pub const ROOT: &str = "/home/uksfta/upload";
/// The shaped fixture port.
pub const PORT: u16 = 2222;
/// The compose file for the shaped bridge fixture.
pub const COMPOSE: &str = "tools/sftp-test-server/docker-compose.shaped.yml";

pub fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn host_key() -> String {
    std::fs::read_to_string(crate_root().join("tools/sftp-test-server/keys/id_ed25519_host.pub"))
        .expect("the fixture host key")
        .trim()
        .to_string()
}

/// True only when both the live and shaped guards are set.
pub fn shaped_enabled() -> bool {
    if std::env::var("UKSFTA_SFTP_LIVE").ok().as_deref() != Some("1") {
        eprintln!("skipped: set UKSFTA_SFTP_LIVE=1 and start the fixture");
        return false;
    }
    if std::env::var("UKSFTA_SFTP_SHAPED").ok().as_deref() != Some("1") {
        eprintln!("skipped: set UKSFTA_SFTP_SHAPED=1 to run against the shaped bridge fixture");
        return false;
    }
    true
}

/// A running shaped container, reached by its bridge address.
pub struct Shaped {
    pub ip: String,
}

impl Shaped {
    /// Start the shaped container and return its address, or the reason the
    /// bridge is not usable on this host.
    pub fn start() -> Result<Self, String> {
        let up = compose(&["up", "-d", "--no-build"]);
        if !up.status.success() {
            return Err(format!(
                "docker compose up failed: {}",
                String::from_utf8_lossy(&up.stderr).trim()
            ));
        }
        let id = compose(&["ps", "-q", "shaped"]);
        let id = String::from_utf8_lossy(&id.stdout).trim().to_string();
        if id.is_empty() {
            return Err("the shaped container has no id".to_string());
        }
        let inspect = Command::new("docker")
            .args([
                "inspect",
                "-f",
                "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}",
                &id,
            ])
            .output()
            .map_err(|error| format!("cannot run docker inspect: {error}"))?;
        let ip = String::from_utf8_lossy(&inspect.stdout).trim().to_string();
        if ip.is_empty() {
            return Err("the shaped container has no bridge address".to_string());
        }
        let address: SocketAddr = format!("{ip}:{PORT}")
            .parse()
            .map_err(|error| format!("bad fixture address {ip}:{PORT}: {error}"))?;
        TcpStream::connect_timeout(&address, Duration::from_secs(3))
            .map_err(|error| format!("cannot reach the shaped fixture at {address}: {error}"))?;

        // Probe the shaper once. A kernel without the netem qdisc cannot
        // shape, and a skip is the correct outcome there.
        let probe = Self { ip };
        let applied = probe.exec_root(&["sh", "/shape.sh", "apply"]);
        if !applied.status.success() {
            return Err(format!(
                "cannot shape the container link: {}",
                String::from_utf8_lossy(&applied.stderr).trim()
            ));
        }
        probe.clear();
        Ok(probe)
    }

    /// The destination the CLI connects to.
    pub fn destination(&self) -> String {
        format!("uksfta@{}", self.ip)
    }

    /// The `[address]:port known_hosts entry.
    pub fn known_hosts_line(&self) -> String {
        format!("[{}]:{PORT} {}\n", self.ip, host_key())
    }

    /// Run one command in the container as the fixture user.
    pub fn exec(&self, args: &[&str]) -> Output {
        Command::new("docker")
            .current_dir(crate_root())
            .args([
                "compose", "-f", COMPOSE, "exec", "-T", "--user", "uksfta", "shaped",
            ])
            .args(args)
            .output()
            .expect("docker compose exec")
    }

    /// Run one command in the container as root, for `tc`.
    pub fn exec_root(&self, args: &[&str]) -> Output {
        Command::new("docker")
            .current_dir(crate_root())
            .args(["compose", "-f", COMPOSE, "exec", "-T", "shaped"])
            .args(args)
            .output()
            .expect("docker compose exec")
    }

    /// Apply the latency and rate shape to the container interface.
    pub fn apply(&self) {
        let out = self.exec_root(&["sh", "/shape.sh", "apply"]);
        assert!(
            out.status.success(),
            "cannot shape the container: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    /// Drop all shaping. This is safe to call more than once.
    pub fn clear(&self) {
        let _ = self.exec_root(&["sh", "/shape.sh", "clear"]);
    }

    /// Set a loss percentage on the shaped interface.
    pub fn loss(&self, percent: u8) {
        let out = self.exec_root(&["sh", "/shape.sh", "loss", &format!("{percent}%")]);
        assert!(
            out.status.success(),
            "cannot set loss: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }

    /// True when no netem qdisc remains on the container interface.
    pub fn qdisc_cleared(&self) -> bool {
        let out = self.exec_root(&["tc", "qdisc", "show", "dev", "eth0"]);
        !String::from_utf8_lossy(&out.stdout).contains("netem")
    }
}

impl Drop for Shaped {
    fn drop(&mut self) {
        self.clear();
    }
}

fn compose(args: &[&str]) -> Output {
    Command::new("docker")
        .current_dir(crate_root())
        .args(["compose", "-f", COMPOSE])
        .args(args)
        .output()
        .expect("docker compose")
}

/// A file of `size` bytes with a repeatable body.
pub fn write_pattern(path: &Path, size: u64) {
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
