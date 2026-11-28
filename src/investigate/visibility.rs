//! Workshop visibility check for resolved origins.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::UksftaError;
use crate::investigate::http::{Http, HttpError, LiveHttp};
use crate::origin::ResolvedOrigin;
use crate::util::{sanitize_for_terminal, workshop_url};
use crate::workshop_api::{build_client, fetch_published_file_details, FileDetail};

/// What the public item page says about an id the API did not describe.
///
/// `ISteamRemoteStorage/GetPublishedFileDetails` omits items it will not
/// index, which covers a removed item, a private one, and an unlisted one.
/// The page separates them: it renders the item when anyone may read it.
#[derive(Debug, PartialEq, Eq)]
enum PageVerdict {
    /// The page renders the item title, so the item exists and is readable
    /// without an account. This is what an unlisted item looks like.
    Visible { title: String },
    /// The page carries the community-guidelines removal notice: withdrawn
    /// from the Workshop, still served to its owner.
    Removed,
    /// The page answers without the item: private to its owner, or gone.
    Unavailable,
    /// Neither shape was found, so the page proves nothing either way.
    Unknown,
}

/// Marker that carries the item name on a Workshop item page.
const TITLE_MARKER: &str = "workshopItemTitle";

/// The page says the item was withdrawn from the Workshop. It can still be
/// served, and still render its title, so this is checked first.
const REMOVED_MARKERS: [&str; 2] = [
    "has been removed from the community",
    "violates steam community &amp; content guidelines",
];

/// The page has no item: Steam serves an error box for a private id, and may
/// redirect to the login page instead.
const UNAVAILABLE_MARKERS: [&str; 3] = [
    "problem accessing the item",
    "this item is private",
    "error_box",
];

/// Query the Workshop API for each mod's visibility, then read the public
/// page for any id the API does not describe, so a removed item and an
/// unlisted one are not reported as the same thing.
pub(crate) fn check_workshop_visibility(results: &[(String, PathBuf, Option<ResolvedOrigin>)]) {
    let mut ids: Vec<String> = results
        .iter()
        .filter_map(|(_, _, origin)| origin.as_ref().map(|o| o.id.clone()))
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
    let by_id: HashMap<&str, &FileDetail> = details
        .iter()
        .map(|detail| (detail.publishedfileid.as_str(), detail))
        .collect();

    let http = LiveHttp::new();
    for id in &ids {
        let detail = by_id.get(id.as_str()).copied();
        if let Some(detail) = detail.filter(|detail| detail.result == 1) {
            let title = page_title(&detail.title).unwrap_or_else(|| id.clone());
            let creator = sanitize_for_terminal(&detail.creator);
            println!("  {id} -> {title} [PUBLIC (by {creator})]");
            continue;
        }
        match probe_page(&http, id) {
            PageVerdict::Visible { title } => println!(
                "  {id} -> {} [UNLISTED (page is public; the item is absent from the API index)]",
                sanitize_for_terminal(&title)
            ),
            PageVerdict::Removed => println!(
                "  {id} -> {id} [REMOVED (the page says it was removed from the community)]"
            ),
            PageVerdict::Unavailable => {
                println!("  {id} -> {id} [REMOVED OR PRIVATE (the page does not load)]")
            }
            PageVerdict::Unknown => println!(
                "  {id} -> {id} [NOT PUBLICLY VISIBLE (no API entry; the page proves nothing)]"
            ),
        }
        if let Some(note) = version_dates(&http, id) {
            println!("      {note}");
        }
    }
}

/// The item's published versions, when the change-notes page lists them.
/// Steam shows one line per upload with a day and time but no year, so the
/// year is not reported. An item with a single version prints nothing.
fn version_dates(http: &dyn Http, id: &str) -> Option<String> {
    let url = format!("https://steamcommunity.com/sharedfiles/filedetails/changelog/{id}");
    let body = http.get(&url).ok()?;
    let dates = changelog_dates(&body);
    let total = changelog_total(&body).unwrap_or(dates.len());
    if total == 0 {
        return None;
    }
    Some(match dates.first() {
        Some(newest) => format!("versions: {total} (newest {newest})"),
        None => format!("versions: {total}"),
    })
}

/// The number of published versions. The pager line counts every one, while
/// one page shows ten.
fn changelog_total(body: &str) -> Option<usize> {
    let start = body.find("Showing ")? + "Showing ".len();
    let line = body[start..].split("entries").next()?;
    let total = line.rsplit(" of ").next()?;
    total.trim().parse().ok()
}

/// The upload dates on a change-notes page, newest first. Each entry starts
/// with `Update: ` and carries the author after `by`.
fn changelog_dates(body: &str) -> Vec<String> {
    let mut dates: Vec<String> = Vec::new();
    for piece in body.split("Update:").skip(1) {
        let date: String = piece
            .split('<')
            .next()
            .unwrap_or("")
            .split("by ")
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if !date.is_empty() && !dates.contains(&date) {
            dates.push(date);
        }
    }
    dates
}

/// One GET of the item page, mapped to what it proves. A transport fault
/// proves nothing, so it never becomes a removal verdict.
fn probe_page(http: &dyn Http, id: &str) -> PageVerdict {
    match http.get(&workshop_url(id)) {
        Ok(body) => parse_item_page(&body),
        Err(HttpError::Status(_)) => PageVerdict::Unavailable,
        Err(_) => PageVerdict::Unknown,
    }
}

/// Read the title block, or the error shape, from an item page.
fn parse_item_page(body: &str) -> PageVerdict {
    let lower = body.to_ascii_lowercase();
    if REMOVED_MARKERS.iter().any(|marker| lower.contains(marker)) {
        return PageVerdict::Removed;
    }
    if UNAVAILABLE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return PageVerdict::Unavailable;
    }
    if let Some(rest) = body.split(TITLE_MARKER).nth(1) {
        if let Some(open) = rest.find('>') {
            let inner = rest[open + 1..].split("</div>").next().unwrap_or("");
            if let Some(title) = page_title(inner) {
                return PageVerdict::Visible { title };
            }
        }
    }
    PageVerdict::Unknown
}

/// Strip markup and entities from an HTML fragment, and return `None` when
/// nothing readable is left.
fn page_title(fragment: &str) -> Option<String> {
    let mut text = String::with_capacity(fragment.len());
    let mut in_tag = false;
    for ch in fragment.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let text = decode_entities(text.trim());
    (!text.is_empty()).then_some(text)
}

/// Decode the entities a Workshop title can carry.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail.find(';').map(|index| index + 1);
        let replacement = end.and_then(|end| match &tail[..end] {
            "&amp;" => Some('&'),
            "&lt;" => Some('<'),
            "&gt;" => Some('>'),
            "&quot;" => Some('"'),
            "&#39;" | "&apos;" => Some('\''),
            _ => None,
        });
        match (replacement, end) {
            (Some(ch), Some(end)) => {
                out.push(ch);
                rest = &tail[end..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeHttp {
        body: Result<String, HttpError>,
    }

    impl Http for FakeHttp {
        fn get(&self, _url: &str) -> Result<String, HttpError> {
            self.body.clone()
        }
    }

    fn fetch(body: Result<&str, HttpError>) -> PageVerdict {
        probe_page(
            &FakeHttp {
                body: body.map(str::to_string),
            },
            "3645230323",
        )
    }

    #[test]
    fn a_rendered_title_means_the_page_is_public() {
        let body = r#"<div class="workshopItemTitle">UKSF Client side</div>"#;
        assert_eq!(
            parse_item_page(body),
            PageVerdict::Visible {
                title: "UKSF Client side".to_string()
            }
        );
        assert_eq!(
            fetch(Ok(body)),
            PageVerdict::Visible {
                title: "UKSF Client side".to_string()
            }
        );
    }

    #[test]
    fn a_title_with_markup_and_entities_is_cleaned() {
        let body =
            r#"<div class="workshopItemTitle workShopContentTitle"><b>UKSF</b> &amp; Co</div>"#;
        assert_eq!(
            parse_item_page(body),
            PageVerdict::Visible {
                title: "UKSF & Co".to_string()
            }
        );
    }

    #[test]
    fn the_error_page_means_removed_or_private() {
        let body = "<div id=\"error_box\">Sorry, there was a problem accessing the item.</div>";
        assert_eq!(parse_item_page(body), PageVerdict::Unavailable);
        assert_eq!(fetch(Ok(body)), PageVerdict::Unavailable);
        assert_eq!(fetch(Err(HttpError::Status(404))), PageVerdict::Unavailable);
    }

    #[test]
    fn a_removal_notice_wins_over_the_rendered_title() {
        let body = r#"<div class="workshopItemTitle">UKSF Client side</div>
            This item has been removed from the community because it violates
            Steam Community &amp; Content Guidelines. It is only visible to you."#;
        assert_eq!(parse_item_page(body), PageVerdict::Removed);
        assert_eq!(fetch(Ok(body)), PageVerdict::Removed);
    }

    #[test]
    fn change_notes_give_the_upload_dates_newest_first() {
        let body = r#"
            Showing 1-4 of 4 entries
            Update: 3 May @ 1:43pm
            by N0v3r
            Update: 3 May @ 7:57am
            by N0v3r
            Update: 12 Feb @ 7:00am
            by N0v3r
        "#;
        assert_eq!(
            changelog_dates(body),
            vec![
                "3 May @ 1:43pm".to_string(),
                "3 May @ 7:57am".to_string(),
                "12 Feb @ 7:00am".to_string()
            ]
        );
        assert!(changelog_dates("<html>no notes</html>").is_empty());
    }

    #[test]
    fn the_pager_gives_the_true_version_count() {
        assert_eq!(changelog_total("Showing 1-10 of 73 entries"), Some(73));
        assert_eq!(changelog_total("Showing 1-4 of 4 entries"), Some(4));
        assert_eq!(changelog_total("no pager here"), None);
    }

    #[test]
    fn a_transport_fault_proves_nothing() {
        assert_eq!(
            fetch(Err(HttpError::Transport("timed out".to_string()))),
            PageVerdict::Unknown
        );
        assert_eq!(
            parse_item_page("<html><body>later</body></html>"),
            PageVerdict::Unknown
        );
    }

    #[test]
    fn an_empty_title_falls_through_to_the_error_shape() {
        let body = r#"<div class="workshopItemTitle"></div><div id="error_box">removed</div>"#;
        assert_eq!(parse_item_page(body), PageVerdict::Unavailable);
    }
}
