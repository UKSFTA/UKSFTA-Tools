//! Shared Steam Workshop API access. One HTTP client policy and one
//! GetPublishedFileDetails request builder for every caller, so the
//! timeout and the form shape cannot drift apart.

use crate::error::UksftaError;

const GET_PUBLISHED_FILE_DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";

/// Build the HTTP client used for every Workshop API call: one 15-second
/// timeout, no automatic retries.
pub fn build_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("Failed to build HTTP client")
}

/// One item from a GetPublishedFileDetails response. Fields an endpoint
/// does not return default to empty.
#[derive(serde::Deserialize)]
pub struct FileDetail {
    pub publishedfileid: String,
    pub result: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subscriptions: u64,
    #[serde(default)]
    pub views: u64,
    #[serde(default)]
    pub favorited: u64,
    #[serde(default)]
    pub time_updated: u64,
    #[serde(default)]
    pub tags: Vec<TagDetail>,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub file_size: String,
}

#[derive(serde::Deserialize)]
#[allow(dead_code)] // `tag` is stored for completeness; only display_name is shown
pub struct TagDetail {
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub display_name: String,
}

#[derive(serde::Deserialize)]
pub struct PublishedFileDetailsResponse {
    response: ResponseInner,
}

#[derive(serde::Deserialize)]
struct ResponseInner {
    publishedfiledetails: Vec<FileDetail>,
}

/// Fetch details for Workshop item ids, 100 per request (the API limit).
/// Returns Err on transport, read, or parse failure so the caller can retry
/// later. A successful response with no items stays Ok.
pub fn fetch_published_file_details(
    ids: &[String],
    client: &reqwest::blocking::Client,
) -> Result<Vec<FileDetail>, UksftaError> {
    let mut details = Vec::new();
    for chunk in ids.chunks(100) {
        let mut form = String::from("itemcount=");
        form.push_str(&chunk.len().to_string());
        for (i, id) in chunk.iter().enumerate() {
            form.push_str(&format!("&publishedfileids[{}]={}", i, id));
        }

        let body = client
            .post(GET_PUBLISHED_FILE_DETAILS_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .ok()
            .and_then(|r| r.text().ok())
            .ok_or_else(|| {
                UksftaError::Input("GetPublishedFileDetails request failed".to_string())
            })?;

        let parsed: PublishedFileDetailsResponse = serde_json::from_str(&body)
            .map_err(|e| UksftaError::Parse(format!("GetPublishedFileDetails response: {e}")))?;
        details.extend(parsed.response.publishedfiledetails);
    }
    Ok(details)
}
