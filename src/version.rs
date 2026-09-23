use std::fs;
use std::path::Path;

/// Query the latest release tag from GitHub (no key required).
fn latest_release_tag() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("Failed to build HTTP client");

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
/// (the automatic check on every command), the GitHub API is queried at
/// most once per day, using a timestamped marker in .uksfta/ so normal
/// commands stay fast and offline-friendly.
pub fn check_for_updates(force: bool) {
    let current = env!("CARGO_PKG_VERSION");
    let marker = Path::new(".uksfta").join("last-update-check");

    if !force {
        // Skip the API call if we already checked today
        if let Ok(mtime) = fs::metadata(&marker).and_then(|m| m.modified()) {
            if let Ok(elapsed) = mtime.elapsed() {
                if elapsed < std::time::Duration::from_secs(24 * 3600) {
                    return;
                }
            }
        }
    }

    let latest = match latest_release_tag() {
        Some(t) => t,
        None => return, // offline or error: stay silent on auto-check
    };

    // Record the check (only when the API succeeded)
    if let Some(dir) = marker.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(&marker, b"");

    if is_outdated(current, latest.trim_start_matches('v')) {
        println!(
            "\nA new version is available: v{} (you have v{})",
            latest.trim_start_matches('v'),
            current
        );
        println!("Run 'uksfta version' for update instructions.");
    }
}

/// Show the installed version and check GitHub for a newer release.
pub fn version() {
    let current = env!("CARGO_PKG_VERSION");
    println!("uksfta {}", current);

    let latest = match latest_release_tag() {
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
}
