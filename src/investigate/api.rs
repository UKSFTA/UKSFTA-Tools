//! Steam Workshop API calls: batch detail lookup and keyed search.

use super::model::ScoredCandidate;
use crate::error::UksftaError;
use crate::util::urlencode;
use crate::workshop_api::{build_client, fetch_published_file_details};

/// Batch-confirm a list of Workshop item IDs via the keyless
/// GetPublishedFileDetails API. Returns ScoredCandidates for the items
/// that exist. Err on transport, read, or parse failure. A successful
/// empty result stays Ok(Vec::new()).
pub(crate) fn batch_workshop_details(
    ids: &[String],
    client: &reqwest::blocking::Client,
) -> Result<Vec<ScoredCandidate>, UksftaError> {
    Ok(fetch_published_file_details(ids, client)?
        .into_iter()
        .filter(|d| d.result == 1)
        .map(|d| ScoredCandidate {
            id: d.publishedfileid,
            title: d.title,
            subscriptions: d.subscriptions,
            views: d.views,
            favorited: d.favorited,
            time_updated: d.time_updated,
            tags: d.tags.into_iter().map(|t| t.display_name).collect(),
            creator: d.creator,
            ..Default::default()
        })
        .collect())
}

/// Search the Workshop via the keyed IPublishedFileService/QueryFiles API.
/// Requires STEAM_API_KEY in the environment. Returns ScoredCandidates
/// with full ranking signals. None when the key is absent or the API
/// call fails, so callers can fall back to the scrape.
pub(crate) fn search_workshop_api(query: &str) -> Option<Vec<ScoredCandidate>> {
    let key = std::env::var("STEAM_API_KEY").ok()?;
    let client = build_client();

    let params = [
        ("key", key.as_str()),
        ("format", "json"),
        ("appid", "107410"),
        ("numperpage", "10"),
        ("query_type", "12"), // k_PublishedFileQueryType_RankedByTextSearch
        ("return_short_description", "1"),
        ("return_tags", "1"),
        ("return_children", "1"),
        ("return_metadata", "1"),
        ("search_text", query),
    ];
    let url = format!(
        "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/?{}",
        urlencode_pairs(&params)
    );

    let body = client.get(&url).send().ok()?.text().ok()?;

    #[derive(serde::Deserialize)]
    struct ApiResponse {
        response: ResponseInner,
    }
    #[derive(serde::Deserialize)]
    struct ResponseInner {
        publishedfiledetails: Vec<FileDetail>,
    }
    #[derive(serde::Deserialize)]
    struct FileDetail {
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
        time_updated: u64,
        #[serde(default)]
        tags: Vec<TagDetail>,
        #[serde(default)]
        creator: String,
        #[serde(default)]
        short_description: String,
        #[serde(default)]
        children: Vec<ChildDetail>,
    }
    #[derive(serde::Deserialize)]
    struct TagDetail {
        #[serde(default)]
        #[allow(dead_code)]
        tag: String,
        #[serde(default)]
        display_name: String,
    }
    #[derive(serde::Deserialize)]
    struct ChildDetail {
        #[serde(default)]
        publishedfileid: String,
    }

    let parsed: ApiResponse = serde_json::from_str(&body).ok()?;
    Some(
        parsed
            .response
            .publishedfiledetails
            .into_iter()
            .map(|d| ScoredCandidate {
                id: d.publishedfileid,
                title: d.title,
                subscriptions: d.subscriptions,
                views: d.views,
                favorited: d.favorited,
                time_updated: d.time_updated,
                tags: d.tags.into_iter().map(|t| t.display_name).collect(),
                creator: d.creator.clone(),
                creator_name: d.creator.clone(), // QueryFiles doesn't return display name
                short_description: d.short_description,
                children: d.children.into_iter().map(|c| c.publishedfileid).collect(),
                ..Default::default()
            })
            .collect(),
    )
}

/// Percent-encode a list of (key, value) pairs for a query string.
pub fn urlencode_pairs(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencode_pairs_encodes_key_values() {
        let pairs = [("search_text", "Zulu Custom"), ("numperpage", "10")];
        assert_eq!(
            urlencode_pairs(&pairs),
            "search_text=Zulu%20Custom&numperpage=10"
        );
    }
}
