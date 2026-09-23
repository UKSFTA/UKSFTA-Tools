//! Workshop visibility check for resolved origins.

use crate::error::UksftaError;
use crate::origin::ResolvedOrigin;
use crate::util::sanitize_for_terminal;
use crate::workshop_api::{build_client, fetch_published_file_details};

/// Query the Steam Workshop API for each investigated mod's visibility.
/// Keyless: ISteamRemoteStorage/GetPublishedFileDetails needs no API key.
pub(crate) fn check_workshop_visibility(results: Vec<(String, Option<ResolvedOrigin>)>) {
    let mut ids: Vec<String> = results
        .iter()
        .filter_map(|(_, origin)| origin.as_ref().map(|o| o.id.clone()))
        .collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        println!("\nOnline check: no origins to check.");
        return;
    }

    println!("\nChecking {} mod(s) against Steam Workshop...", ids.len());
    let client = build_client();

    let details = match fetch_published_file_details(&ids, &client) {
        Ok(details) => details,
        Err(e) => {
            if matches!(e, UksftaError::Parse(_)) {
                eprintln!("Error parsing API response: {}", e);
            } else {
                eprintln!("Error calling Steam API: {}", e);
            }
            return;
        }
    };

    for detail in details {
        let status = if detail.result == 1 {
            format!("PUBLIC (by {})", detail.creator)
        } else {
            "NOT PUBLICLY VISIBLE (removed or private)".to_string()
        };
        let title = if detail.title.is_empty() {
            detail.publishedfileid.clone()
        } else {
            sanitize_for_terminal(&detail.title)
        };
        println!("  {} -> {} [{}]", detail.publishedfileid, title, status);
    }
}
