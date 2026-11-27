//! The read-only preflight behind `uksfta server --doctor`.
//!
//! It resolves the settings, checks the known_hosts file, lists the pack
//! folder with one batch and asks for free space. It never plans, verifies,
//! uploads or creates a directory.

use std::fs::File;
use std::time::{Duration, Instant};

use super::config::Settings;
use super::hostkey::HostKeyPolicy;
use super::parse;
use super::sftp::{quote, Sftp};
use crate::error::UksftaError;

/// One doctor result.
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// What the probe found.
pub struct Probe {
    pub checks: Vec<Check>,
    pub free_space: Option<u64>,
    pub largest: Option<u64>,
    pub connect_ms: u64,
}

/// The effective settings, one line each.
pub fn settings_lines(settings: &Settings) -> Vec<String> {
    let readable = File::open(&settings.known_hosts).is_ok();
    vec![
        format!("doctor: destination {}", settings.destination),
        format!("doctor: port {}", settings.port),
        format!("doctor: root {}", settings.root),
        format!(
            "doctor: known_hosts {} ({})",
            settings.known_hosts.display(),
            if readable { "readable" } else { "missing" }
        ),
        format!(
            "doctor: host_key_policy {}",
            settings.host_key_policy.as_str()
        ),
        format!("doctor: buffer {}", settings.transfer.buffer),
        format!("doctor: requests {}", settings.transfer.requests),
    ]
}

/// Probe the server. A failed connection is a check, not an error, so the
/// caller can still report the settings.
pub fn probe(settings: &Settings, pack_dir: Option<&str>) -> Probe {
    let mut checks = Vec::new();
    let readable = File::open(&settings.known_hosts).is_ok();
    checks.push(Check {
        name: "known_hosts",
        ok: readable || settings.host_key_policy == HostKeyPolicy::AcceptNew,
        detail: if readable {
            settings.known_hosts.display().to_string()
        } else {
            format!(
                "missing or unreadable at {}",
                settings.known_hosts.display()
            )
        },
    });

    let started = Instant::now();
    let Some(pack_dir) = pack_dir else {
        checks.push(Check {
            name: "connect",
            ok: false,
            detail: "no pack folder to list".to_string(),
        });
        return Probe {
            checks,
            free_space: None,
            largest: None,
            connect_ms: millis(started.elapsed()),
        };
    };

    let client = Sftp::new(settings);
    let (connect, pack, largest) = match list(&client, settings, pack_dir) {
        Ok((files, largest)) => (
            Check {
                name: "connect",
                ok: true,
                detail: "pwd and listing succeeded".to_string(),
            },
            Check {
                name: "pack",
                ok: true,
                detail: format!("{files} file(s)"),
            },
            Some(largest),
        ),
        Err(error) => (
            Check {
                name: "connect",
                ok: false,
                detail: error.to_string(),
            },
            Check {
                name: "pack",
                ok: false,
                detail: format!("{pack_dir}: not listed"),
            },
            None,
        ),
    };
    let free_space = if connect.ok {
        free_space(&client, pack_dir)
    } else {
        None
    };
    checks.push(connect);
    checks.push(pack);
    Probe {
        checks,
        free_space,
        largest,
        connect_ms: millis(started.elapsed()),
    }
}

/// The effective segmented settings, or the not-requested line.
pub fn segment_line(settings: &Settings) -> String {
    if !settings.segment.enabled {
        return "segment: not requested".to_string();
    }
    let chunk = settings
        .segment
        .chunk
        .map_or_else(|| "auto".to_string(), |chunk| chunk.to_string());
    format!(
        "segment: min {} bytes, {} connections, chunk {chunk}, resume {}, {} reconnects",
        settings.segment.min_bytes,
        settings.segment.connections,
        settings.segment.resume.as_str(),
        settings.segment.reconnects
    )
}

/// The largest file in the pack listing against the minimum.
pub fn segment_size_line(settings: &Settings, largest: Option<u64>) -> String {
    match largest {
        Some(bytes) if bytes >= settings.segment.min_bytes => format!(
            "segment: largest {bytes} bytes, takes the segmented path (min {} bytes)",
            settings.segment.min_bytes
        ),
        Some(bytes) => format!(
            "segment: largest {bytes} bytes, below the min {} bytes",
            settings.segment.min_bytes
        ),
        None => "segment: largest unknown (no pack listing)".to_string(),
    }
}

fn list(client: &Sftp, settings: &Settings, pack_dir: &str) -> Result<(usize, u64), UksftaError> {
    let lines = vec![
        format!("cd {}", quote(&settings.root)?),
        format!("-cd {}", quote(pack_dir)?),
        "pwd".to_string(),
        "ls -l".to_string(),
    ];
    let blocks = parse::parse_blocks(&client.batch(&lines)?);
    let block = parse::find_block(&blocks, pack_dir)
        .ok_or_else(|| UksftaError::Input(format!("no pack folder at {pack_dir}")))?;
    let files: Vec<_> = block
        .entries
        .iter()
        .filter(|entry| !entry.is_dir && !entry.is_symlink)
        .collect();
    let largest = files
        .iter()
        .map(|entry| entry.size)
        .max()
        .unwrap_or_default();
    Ok((files.len(), largest))
}

/// Free space on the server, best effort. No `df` support is `None`.
fn free_space(client: &Sftp, pack_dir: &str) -> Option<u64> {
    let lines = vec![format!("-cd {}", quote(pack_dir).ok()?), "df".to_string()];
    let text = client.batch(&lines).ok()?;
    text.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let _size = fields.next()?.parse::<u64>().ok()?;
        let _used = fields.next()?.parse::<u64>().ok()?;
        fields.next()?.parse::<u64>().ok()
    })
}

fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}
