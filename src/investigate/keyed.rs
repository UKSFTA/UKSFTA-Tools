//! Opt-in keyed Workshop search through `IPublishedFileService/QueryFiles`.
//!
//! The keyed API returns Steam's ranked title search, which the keyless
//! browse page sometimes buries. It is a candidate source only: a result
//! still has to pass the evidence rules in [`super::resolve`] before it
//! names a PBO, and nothing here fetches an item or a manifest.
//!
//! The API key is read from the environment, held in a private field, and
//! never printed: [`LiveKeyedSearch`] has a redacting `Debug`, and every
//! [`KeyedError`] message is built without the key or the request URL.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::util::urlencode;

use super::browse::{BrowsePage, BrowseResult};

/// The environment variable that carries the API key.
pub(crate) const STEAM_API_KEY_ENV: &str = "STEAM_API_KEY";

const QUERY_FILES_URL: &str = "https://api.steampowered.com/IPublishedFileService/QueryFiles/v1/";

/// Arma 3, the only app this tool searches.
const ARMA_APP_ID: u32 = 107_410;

/// Largest page the search asks for, so one query is one response.
const PAGE_SIZE: u32 = 50;

/// `k_PublishedFileQueryType_RankedByTextSearch`.
const RANKED_BY_TEXT_SEARCH: u32 = 12;

/// The reciprocal-rank constant. A hit in both of two queries scores
/// higher than a hit in one, and the position still orders the rest.
const RANK_FUSION_K: f64 = 60.0;

/// Why a keyed search could not answer. The variant is the whole error:
/// no URL, body or key text is kept, so no message can leak the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyedError {
    /// `STEAM_API_KEY` is unset or empty.
    MissingKey,
    /// HTTP 403: Steam refused the key.
    Rejected,
    /// HTTP 429: the key is valid but the endpoint is throttling.
    RateLimited,
    /// Any other unexpected status.
    Status(u16),
    /// The request or the body did not complete.
    Transport,
    /// The body was not a QueryFiles response.
    Parse,
}

impl KeyedError {
    /// The operator-facing sentence. It never includes the key.
    pub(crate) fn message(&self) -> String {
        match self {
            KeyedError::MissingKey => format!(
                "--steam-search requires {STEAM_API_KEY_ENV}. Set it in the environment, \
                 or run without --steam-search."
            ),
            KeyedError::Rejected => format!(
                "Steam refused the Workshop search: {STEAM_API_KEY_ENV} was rejected \
                 (HTTP 403). Check the key, or run without --steam-search."
            ),
            KeyedError::RateLimited => {
                "Steam rate-limited the Workshop search (HTTP 429); retry later.".to_string()
            }
            KeyedError::Status(code) => format!("Steam Workshop search failed with HTTP {code}."),
            KeyedError::Transport => "Steam Workshop search request failed.".to_string(),
            KeyedError::Parse => "Steam Workshop search response was not readable.".to_string(),
        }
    }

    /// A bad key is a configuration fault, not a search miss, so the run
    /// stops instead of reporting every PBO unresolved.
    pub(crate) fn is_fatal(&self) -> bool {
        matches!(self, KeyedError::MissingKey | KeyedError::Rejected)
    }
}

/// One keyed search source. The trait exists so a test supplies a fixed
/// response and never opens a socket.
pub(crate) trait KeyedSearch {
    /// One `QueryFiles` request for `text`, results in served rank order.
    fn query(&self, text: &str) -> Result<BrowsePage, KeyedError>;
}

/// The live source, with the key held privately.
pub(crate) struct LiveKeyedSearch {
    client: reqwest::blocking::Client,
    key: String,
}

impl std::fmt::Debug for LiveKeyedSearch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LiveKeyedSearch")
            .field("key", &"[redacted]")
            .finish_non_exhaustive()
    }
}

impl LiveKeyedSearch {
    /// Build the source from the environment. An absent or blank key is
    /// [`KeyedError::MissingKey`]; the caller must not fall back silently.
    pub(crate) fn from_env() -> Result<Self, KeyedError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, KeyedError> {
        let key = lookup(STEAM_API_KEY_ENV)
            .map(|key| key.trim().to_string())
            .filter(|key| !key.is_empty())
            .ok_or(KeyedError::MissingKey)?;
        Ok(Self {
            client: crate::workshop_api::build_client(),
            key,
        })
    }

    fn request_body(&self, text: &str) -> String {
        format!(
            "key={}&format=json&appid={ARMA_APP_ID}&query_type={RANKED_BY_TEXT_SEARCH}\
             &numperpage={PAGE_SIZE}&return_short_description=false&search_text={}",
            urlencode(&self.key),
            urlencode(text)
        )
    }
}

impl KeyedSearch for LiveKeyedSearch {
    fn query(&self, text: &str) -> Result<BrowsePage, KeyedError> {
        let response = self
            .client
            .post(QUERY_FILES_URL)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(self.request_body(text))
            .send()
            .map_err(|_| KeyedError::Transport)?;
        // The key is in the body, never in the URL, so no status or
        // transport error can echo it.
        match response.status().as_u16() {
            200..=299 => {}
            403 => return Err(KeyedError::Rejected),
            429 => return Err(KeyedError::RateLimited),
            code => return Err(KeyedError::Status(code)),
        }
        let body = response.text().map_err(|_| KeyedError::Transport)?;
        parse_page(&body)
    }
}

#[derive(Deserialize)]
struct ApiResponse {
    response: ResponseInner,
}

#[derive(Deserialize)]
struct ResponseInner {
    #[serde(default)]
    total: u64,
    #[serde(default)]
    publishedfiledetails: Vec<Detail>,
}

#[derive(Deserialize)]
struct Detail {
    publishedfileid: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    creator: String,
    #[serde(default)]
    subscriptions: u64,
    #[serde(default)]
    lifetime_subscriptions: u64,
    #[serde(default)]
    views: u64,
    #[serde(default)]
    favorited: u64,
    #[serde(default)]
    time_created: u64,
    #[serde(default)]
    time_updated: u64,
    #[serde(default, deserialize_with = "size_text")]
    file_size: String,
    #[serde(default)]
    file_type: u32,
    #[serde(default)]
    tags: Vec<Tag>,
    /// Zero means the item was removed between the index and this call.
    #[serde(default)]
    result: Option<u32>,
}

#[derive(Deserialize)]
struct Tag {
    #[serde(default)]
    tag: String,
}

/// Steam returns `file_size` as a string on some responses and a number
/// on others. Both become the text the browse parser produces.
fn size_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Size {
        Text(String),
        Number(u64),
    }
    Ok(match Option::<Size>::deserialize(deserializer)? {
        Some(Size::Text(text)) => text,
        Some(Size::Number(number)) => number.to_string(),
        None => String::new(),
    })
}

/// Read one QueryFiles body into the page shape the resolver already uses.
fn parse_page(body: &str) -> Result<BrowsePage, KeyedError> {
    let parsed: ApiResponse = serde_json::from_str(body).map_err(|_| KeyedError::Parse)?;
    let results: Vec<BrowseResult> = parsed
        .response
        .publishedfiledetails
        .into_iter()
        .filter(|detail| detail.result.unwrap_or(1) == 1)
        .map(|detail| BrowseResult {
            id: detail.publishedfileid,
            title: detail.title,
            // The request asks for no description, so nothing is invented.
            short_description: String::new(),
            tags: detail.tags.into_iter().map(|tag| tag.tag).collect(),
            subscriptions: detail.subscriptions,
            lifetime_subscriptions: detail.lifetime_subscriptions,
            views: detail.views,
            favorited: detail.favorited,
            time_created: detail.time_created,
            time_updated: detail.time_updated,
            file_size: detail.file_size,
            num_children: 0,
            children: Vec::new(),
            star_rating: 0.0,
            total_votes: 0,
            creator: detail.creator,
            file_type: detail.file_type,
        })
        .collect();
    let total_pages = results.len().div_ceil(PAGE_SIZE as usize) as u32;
    Ok(BrowsePage {
        current_page: 1,
        total_pages,
        total_count: parsed.response.total.max(results.len() as u64),
        results,
    })
}

/// Merge per-query rankings into one list, one entry per item.
///
/// An item two queries returned outranks an item one query returned.
/// Within the same support the better served position wins, and equals
/// fall back to id so the order never depends on request order.
pub(crate) fn merge_ranked(pages: &[BrowsePage]) -> Vec<BrowseResult> {
    struct Fused {
        score: f64,
        best_position: usize,
        item: BrowseResult,
    }

    let mut fused: BTreeMap<String, Fused> = BTreeMap::new();
    for page in pages {
        for (position, item) in page.results.iter().enumerate() {
            let score = 1.0 / (RANK_FUSION_K + position as f64);
            match fused.get_mut(&item.id) {
                Some(existing) => {
                    existing.score += score;
                    existing.best_position = existing.best_position.min(position);
                }
                None => {
                    fused.insert(
                        item.id.clone(),
                        Fused {
                            score,
                            best_position: position,
                            item: item.clone(),
                        },
                    );
                }
            }
        }
    }

    let mut ranked: Vec<Fused> = fused.into_values().collect();
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.best_position.cmp(&right.best_position))
            .then_with(|| left.item.id.cmp(&right.item.id))
    });
    ranked.into_iter().map(|fused| fused.item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/queryfiles.json");

    fn item(id: &str, title: &str) -> BrowseResult {
        BrowseResult {
            id: id.to_string(),
            title: title.to_string(),
            short_description: String::new(),
            tags: Vec::new(),
            subscriptions: 0,
            lifetime_subscriptions: 0,
            views: 0,
            favorited: 0,
            time_created: 0,
            time_updated: 0,
            file_size: String::new(),
            num_children: 0,
            children: Vec::new(),
            star_rating: 0.0,
            total_votes: 0,
            creator: String::new(),
            file_type: 0,
        }
    }

    fn page(ids: &[(&str, &str)]) -> BrowsePage {
        BrowsePage {
            current_page: 1,
            total_pages: 1,
            total_count: ids.len() as u64,
            results: ids.iter().map(|(id, title)| item(id, title)).collect(),
        }
    }

    #[test]
    fn parse_reads_the_query_files_shape() {
        let parsed = parse_page(FIXTURE).unwrap();

        assert_eq!(parsed.results.len(), 3);
        assert_eq!(parsed.results[0].id, "450814997");
        assert_eq!(parsed.results[0].title, "CBA_A3");
        assert_eq!(parsed.results[0].creator, "ACE Team");
        assert_eq!(parsed.results[0].file_size, "12345678");
        assert_eq!(parsed.results[0].tags, ["Mod", "Arma 3"]);
        assert_eq!(parsed.total_count, 412);
        assert_eq!(parsed.total_pages, 1);
    }

    #[test]
    fn parse_drops_removed_items_and_accepts_a_numeric_size() {
        let body = r#"{"response":{"total":2,"publishedfiledetails":[
            {"publishedfileid":"1","title":"Gone","result":9},
            {"publishedfileid":"2","title":"Here","file_size":4096,"result":1}
        ]}}"#;

        let parsed = parse_page(body).unwrap();

        assert_eq!(parsed.results.len(), 1);
        assert_eq!(parsed.results[0].id, "2");
        assert_eq!(parsed.results[0].file_size, "4096");
    }

    #[test]
    fn parse_rejects_a_body_that_is_not_a_response() {
        assert_eq!(parse_page("<html>"), Err(KeyedError::Parse));
    }

    #[test]
    fn merge_keeps_a_single_query_in_served_order() {
        let merged = merge_ranked(&[page(&[("1", "A"), ("2", "B"), ("3", "C")])]);

        assert_eq!(
            merged
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
    }

    #[test]
    fn merge_deduplicates_and_ranks_agreement_first() {
        let first = page(&[("1", "A"), ("2", "B"), ("3", "C")]);
        let second = page(&[("4", "D"), ("2", "B"), ("5", "E")]);

        let merged = merge_ranked(&[first, second]);

        let ids: Vec<&str> = merged.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, ["2", "1", "4", "3", "5"]);
        assert_eq!(merged.iter().filter(|item| item.id == "2").count(), 1);
    }

    #[test]
    fn missing_key_names_the_variable() {
        let result = LiveKeyedSearch::from_lookup(|_| None);

        let error = result.unwrap_err();
        assert_eq!(error, KeyedError::MissingKey);
        assert!(error.message().contains(STEAM_API_KEY_ENV));
        assert!(error.is_fatal());
    }

    #[test]
    fn blank_key_is_missing() {
        let result = LiveKeyedSearch::from_lookup(|_| Some("   ".to_string()));

        assert_eq!(result.unwrap_err(), KeyedError::MissingKey);
    }

    #[test]
    fn bad_key_names_the_variable_and_is_fatal() {
        let error = KeyedError::Rejected;

        assert!(error.message().contains(STEAM_API_KEY_ENV));
        assert!(error.message().contains("403"));
        assert!(error.is_fatal());
        assert!(!KeyedError::RateLimited.is_fatal());
    }

    #[test]
    fn a_failure_path_never_shows_the_key() {
        const KEY: &str = "0123456789abcdef0123456789abcdef";
        let search = LiveKeyedSearch {
            client: crate::workshop_api::build_client(),
            key: KEY.to_string(),
        };

        let shown = format!(
            "{search:?} {} {} {}",
            KeyedError::Rejected.message(),
            KeyedError::MissingKey.message(),
            KeyedError::Status(500).message()
        );

        assert!(!shown.contains(KEY));
        assert!(shown.contains("[redacted]"));
    }

    #[test]
    fn request_carries_the_key_in_the_body_only() {
        let search = LiveKeyedSearch {
            client: crate::workshop_api::build_client(),
            key: "abc".to_string(),
        };

        let body = search.request_body("ADE diving");

        assert!(body.contains("key=abc"));
        assert!(body.contains("search_text=ADE%20diving"));
        assert!(body.contains("numperpage=50"));
        assert!(body.contains("return_short_description=false"));
        assert!(!QUERY_FILES_URL.contains("key="));
    }

    #[test]
    #[ignore = "live Steam call; run with --ignored and set UKSFTA_STEAM_LIVE=1 plus STEAM_API_KEY"]
    fn live_search_returns_ranked_results() {
        if std::env::var("UKSFTA_STEAM_LIVE").ok().as_deref() != Some("1") {
            return;
        }
        let search = LiveKeyedSearch::from_env().expect("STEAM_API_KEY must be set");
        let page = search
            .query("Advanced Diving Environment")
            .expect("the keyed search must answer");

        assert!(!page.results.is_empty());
        assert!(page.results.iter().all(|item| !item.id.is_empty()));
    }
}
