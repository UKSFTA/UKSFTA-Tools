//! Keyless Workshop browse-page scrape and SSR blob parsing.

use super::api::batch_workshop_details;
use super::model::ScoredCandidate;
use crate::error::UksftaError;
use crate::util::urlencode;
use crate::workshop_api::build_client;

/// Search the Steam Workshop browse page for a query and return candidate
/// ScoredCandidates. Parses the SSR JSON blob for rich data (tags,
/// subscriptions, creator name, etc.) instead of just extracting IDs.
/// Returns Err on transport or read failure. A successful empty result
/// stays Ok(Vec::new()).
pub(crate) fn search_workshop(query: &str) -> Result<Vec<ScoredCandidate>, UksftaError> {
    let client = build_client();
    let url = format!(
        "https://steamcommunity.com/workshop/browse/?appid=107410&searchtext={}&requiredtags[]=Mod",
        urlencode(query)
    );
    let html = match client.get(&url).send().ok().and_then(|r| r.text().ok()) {
        Some(h) => h,
        None => {
            return Err(UksftaError::Input(
                "Workshop browse request failed".to_string(),
            ))
        }
    };

    // Try to extract the SSR JSON blob first (rich data).
    if let Some(candidates) = parse_ssr_blob(&html) {
        return Ok(candidates);
    }

    // Fallback: regex ID extraction (original approach, no ranking data).
    // Enrich via batch details so titles and stats are available.
    let mut ids = Vec::new();
    for id in html.split("filedetails/?id=").skip(1) {
        let id: String = id.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !id.is_empty() && !ids.contains(&id) {
            ids.push(id);
            if ids.len() >= 10 {
                break;
            }
        }
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    batch_workshop_details(&ids, &client)
}

/// Parse the `window.SSR.renderContext` JSON blob from the Workshop
/// browse page. This blob contains 30 items per page with full metadata
/// (tags, subscriptions, creator name, star rating, etc.) that the
/// regex approach misses entirely.
fn parse_ssr_blob(html: &str) -> Option<Vec<ScoredCandidate>> {
    // The blob is embedded as: window.SSR.renderContext=JSON.parse("...")
    let prefix = "window.SSR.renderContext=JSON.parse(\"";
    let start = html.find(prefix)? + prefix.len();
    let end = html[start..].find("\");")? + start;
    let escaped = &html[start..end];

    // Unescape the JSON string: \" -> ", \\ -> \
    let json_str = escaped.replace("\\\"", "\"").replace("\\\\", "\\");

    #[derive(serde::Deserialize)]
    struct SsrContext {
        #[serde(rename = "workshop_browse")]
        workshop_browse: Option<WorkshopBrowse>,
    }
    #[derive(serde::Deserialize)]
    struct WorkshopBrowse {
        results: Vec<SsrItem>,
    }
    #[derive(serde::Deserialize)]
    struct SsrItem {
        #[serde(default)]
        publishedfileid: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        subscriptions: u64,
        #[serde(default)]
        views: u64,
        #[serde(default)]
        favorited: u64,
        #[serde(default)]
        star_rating: f64,
        #[serde(default)]
        total_votes: u32,
        #[serde(default)]
        tags: Vec<SsrTag>,
        #[serde(default)]
        creator: String,
        #[serde(default)]
        short_description: String,
        #[serde(default)]
        time_updated: u64,
        #[serde(default)]
        file_type: u32,
        #[serde(default)]
        creator_player_link_details: Option<CreatorDetails>,
        #[serde(default)]
        children: Option<Vec<SsrChild>>,
    }
    #[derive(serde::Deserialize)]
    #[allow(dead_code)]
    struct SsrTag {
        #[serde(default)]
        tag: String,
        #[serde(default)]
        display_name: String,
    }
    #[derive(serde::Deserialize)]
    struct CreatorDetails {
        #[serde(default)]
        persona_name: String,
    }
    #[derive(serde::Deserialize)]
    struct SsrChild {
        #[serde(default)]
        publishedfileid: String,
    }

    let ctx: SsrContext = serde_json::from_str(&json_str).ok()?;
    let browse = ctx.workshop_browse?;
    let mut results = Vec::new();
    for item in browse.results {
        results.push(ScoredCandidate {
            id: item.publishedfileid,
            title: item.title,
            search_score: 0.0, // SSR blob has no search_score field
            subscriptions: item.subscriptions,
            views: item.views,
            favorited: item.favorited,
            star_rating: item.star_rating,
            total_votes: item.total_votes,
            tags: item.tags.into_iter().map(|t| t.display_name).collect(),
            creator: item.creator.clone(),
            creator_name: item
                .creator_player_link_details
                .map(|d| d.persona_name)
                .unwrap_or_default(),
            time_updated: item.time_updated,
            short_description: item.short_description,
            children: item
                .children
                .unwrap_or_default()
                .into_iter()
                .map(|c| c.publishedfileid)
                .collect(),
            file_type: item.file_type,
        });
    }
    Some(results)
}
