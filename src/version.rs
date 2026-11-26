use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a successful check stays fresh.
const SUCCESS_TTL: Duration = Duration::from_secs(24 * 3600);
/// How long a failed check stays fresh.
const FAILURE_TTL: Duration = Duration::from_secs(3600);

/// Cache window for one recorded check outcome.
const fn check_ttl(ok: bool) -> Duration {
    if ok {
        SUCCESS_TTL
    } else {
        FAILURE_TTL
    }
}

/// True when a recorded marker is still inside its cache window. A missing
/// marker is never fresh, so the check runs.
fn marker_is_fresh(marker: Option<(Duration, bool)>) -> bool {
    match marker {
        Some((age, ok)) => age < check_ttl(ok),
        None => false,
    }
}

/// The marker file: a per-user cache path, never the current directory.
/// A machine with no cache directory falls back to the temp directory.
fn marker_path() -> PathBuf {
    dirs::cache_dir()
        .map(|dir| dir.join("uksfta").join("last-update-check"))
        .unwrap_or_else(|| std::env::temp_dir().join("uksfta-last-update-check"))
}

/// Read the marker as `(age, ok)`. Missing or unreadable means "no record".
fn read_marker(path: &Path) -> Option<(Duration, bool)> {
    let modified = fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let age = modified.elapsed().ok()?;
    let content = fs::read_to_string(path).ok()?;
    Some((age, content.trim() == "ok"))
}

/// Record the outcome of a check so the next command can skip the network.
/// A write failure is not an error: the next command checks again.
fn record_marker(path: &Path, ok: bool) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(path, if ok { "ok" } else { "fail" });
}

/// Query the latest release tag from GitHub (no key required).
fn latest_release_tag() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;

    let body = client
        .get("https://api.github.com/repos/UKSFTA/UKSFTA-Tools/releases/latest")
        .header("User-Agent", "uksfta")
        .send()
        .ok()?
        .text()
        .ok()?;

    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release = serde_json::from_str(&body).ok()?;
    Some(release.tag_name)
}

/// Print an update notice if a newer release exists. When `force` is false
/// (the automatic check on every command), the GitHub API is queried only
/// when the cached outcome is stale: 24 hours after a success, one hour
/// after a failure. The marker lives in the user cache directory, so
/// normal commands stay fast, work offline, and write nothing to the
/// current directory.
pub fn check_for_updates(force: bool) {
    let marker = marker_path();

    if !force && marker_is_fresh(read_marker(&marker)) {
        return;
    }

    let current = env!("CARGO_PKG_VERSION");
    let latest = latest_release_tag();
    record_marker(&marker, latest.is_some());

    if let Some(latest) = latest {
        if is_outdated(current, latest.trim_start_matches('v')) {
            println!(
                "\nA new version is available: v{} (you have v{})",
                latest.trim_start_matches('v'),
                current
            );
            println!("Run 'uksfta version' for update instructions.");
        }
    }
}

/// Show the installed version and check GitHub for a newer release.
pub fn version() {
    let current = env!("CARGO_PKG_VERSION");
    println!("uksfta {}", current);

    let latest = latest_release_tag();
    record_marker(&marker_path(), latest.is_some());
    let latest = match latest {
        Some(t) => t.trim_start_matches('v').to_string(),
        None => {
            println!("Could not check for updates (offline or no access to GitHub).");
            return;
        }
    };

    if !is_outdated(current, &latest) {
        println!("You are up to date.");
        return;
    }

    println!(
        "\nA new version is available: v{} (you have v{})",
        latest, current
    );
    println!("Update with:");
    if cfg!(target_os = "windows") {
        println!(
            "  irm https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.ps1 | iex"
        );
    } else {
        println!(
            "  curl -fsSL https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.sh | sh"
        );
    }
}

/// Compare two dotted version strings. Returns true when `installed` is
/// older than `latest`. Handles the "v" prefix. Non-numeric parts are
/// ignored for the comparison.
fn is_outdated(installed: &str, latest: &str) -> bool {
    let installed: Vec<u32> = installed
        .trim_start_matches('v')
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();
    let latest: Vec<u32> = latest
        .trim_start_matches('v')
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect();

    for (a, b) in installed.iter().zip(latest.iter()) {
        if a != b {
            return a < b;
        }
    }
    installed.len() < latest.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_outdated_detects_newer_major() {
        assert!(is_outdated("0.1.0", "0.2.0"));
        assert!(is_outdated("1.0.0", "2.0.0"));
    }

    #[test]
    fn is_outdated_detects_newer_patch() {
        assert!(is_outdated("0.2.0", "0.2.1"));
    }

    #[test]
    fn is_outdated_same_version_is_false() {
        assert!(!is_outdated("0.2.0", "0.2.0"));
        assert!(!is_outdated("v0.2.0", "0.2.0"));
    }

    #[test]
    fn is_outdated_newer_installed_is_false() {
        assert!(!is_outdated("0.3.0", "0.2.0"));
    }

    #[test]
    fn is_outdated_handles_v_prefix_and_trimmed() {
        // The auto-check passes trimmed tags; version passes raw
        assert!(!is_outdated("0.4.0", "0.4.0"));
        assert!(is_outdated("0.3.0", "0.4.0"));
        assert!(is_outdated("0.3.0", "0.4.1"));
    }

    #[test]
    fn check_ttl_is_one_day_after_success_and_one_hour_after_failure() {
        assert_eq!(check_ttl(true), Duration::from_secs(24 * 3600));
        assert_eq!(check_ttl(false), Duration::from_secs(3600));
    }

    #[test]
    fn marker_is_fresh_within_the_success_window() {
        assert!(marker_is_fresh(Some((
            Duration::from_secs(23 * 3600),
            true
        ))));
        assert!(!marker_is_fresh(Some((
            Duration::from_secs(25 * 3600),
            true
        ))));
    }

    #[test]
    fn marker_is_fresh_within_the_failure_window() {
        assert!(marker_is_fresh(Some((Duration::from_secs(1800), false))));
        assert!(!marker_is_fresh(Some((
            Duration::from_secs(2 * 3600),
            false
        ))));
    }

    #[test]
    fn marker_is_fresh_is_false_when_the_marker_is_missing() {
        assert!(!marker_is_fresh(None));
    }
}
