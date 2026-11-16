//! Parser for the Steam Workshop browse page.
//!
//! The page serves its ranked data inside
//! `<script type="application/json" id="valve-ssr-data">`. The outer JSON
//! carries `renderContext.queryData` as a JSON *string*; parsing it again
//! yields `queries[]`, and the entry whose `queryKey` starts with
//! `workshop_browse` holds `state.data` with the results in served rank
//! order. A page without that script is an error: the sidebar
//! `filedetails/?id=` links are never read as results.

use serde::Deserialize;

use super::http::{fetch_with_retry, Endpoint, Http, HttpError, RateLimiter};

const BROWSE_URL: &str = "https://steamcommunity.com/workshop/browse/";
const SSR_ID_ATTRIBUTE: &str = "id=\"valve-ssr-data\"";
const SCRIPT_CLOSE: &str = "</script>";
const BROWSE_QUERY_KEY: &str = "workshop_browse";

/// One Workshop item, as listed on the browse page.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BrowseResult {
    pub id: String,
    pub title: String,
    pub short_description: String,
    pub tags: Vec<String>,
    pub subscriptions: u64,
    pub lifetime_subscriptions: u64,
    pub views: u64,
    pub favorited: u64,
    pub time_created: u64,
    pub time_updated: u64,
    pub file_size: String,
    pub num_children: u32,
    pub children: Vec<String>,
    pub star_rating: f64,
    pub total_votes: u32,
    pub creator: String,
    /// `EWorkshopFileType`; 0 is a mod. Collections and other types are
    /// not candidates for a PBO's origin.
    pub file_type: u32,
}

/// One browse page, results in the order Steam served them.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BrowsePage {
    pub current_page: u32,
    pub total_pages: u32,
    pub total_count: u64,
    pub results: Vec<BrowseResult>,
}

/// Why a page could not be read. Each variant is a distinct payload
/// shape, so a caller can tell a changed format from a bad query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BrowseParseError {
    MissingSsrScript,
    BadScriptJson(String),
    QueryDataIsNotJson(String),
    NoBrowseQuery,
}

/// Why a browse search failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchError {
    Http(HttpError),
    Parse(BrowseParseError),
}

impl From<HttpError> for SearchError {
    fn from(error: HttpError) -> Self {
        SearchError::Http(error)
    }
}

impl From<BrowseParseError> for SearchError {
    fn from(error: BrowseParseError) -> Self {
        SearchError::Parse(error)
    }
}

#[derive(Deserialize, Default)]
struct SsrEnvelope {
    #[serde(default, rename = "renderContext")]
    render_context: Option<SsrRenderContext>,
}

#[derive(Deserialize, Default)]
struct SsrRenderContext {
    #[serde(default, rename = "queryData")]
    query_data: Option<String>,
}

#[derive(Deserialize, Default)]
struct QueryData {
    #[serde(default)]
    queries: Vec<Query>,
}

#[derive(Deserialize, Default)]
struct Query {
    #[serde(default, rename = "queryKey")]
    query_key: Vec<serde_json::Value>,
    #[serde(default)]
    state: Option<QueryState>,
}

#[derive(Deserialize, Default)]
struct QueryState {
    #[serde(default)]
    data: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BrowseData {
    current_page: u32,
    total_pages: u32,
    total_count: u64,
    results: Vec<RawResult>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawResult {
    publishedfileid: String,
    title: String,
    short_description: String,
    creator: String,
    tags: Vec<RawTag>,
    subscriptions: u64,
    lifetime_subscriptions: u64,
    views: u64,
    favorited: u64,
    time_created: u64,
    time_updated: u64,
    file_size: String,
    num_children: u32,
    children: Vec<String>,
    star_rating: f64,
    total_votes: u32,
    file_type: u32,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawTag {
    display_name: String,
}

impl From<RawResult> for BrowseResult {
    fn from(raw: RawResult) -> Self {
        BrowseResult {
            id: raw.publishedfileid,
            title: raw.title,
            short_description: raw.short_description,
            tags: raw
                .tags
                .into_iter()
                .map(|tag| tag.display_name)
                .filter(|name| !name.is_empty())
                .collect(),
            subscriptions: raw.subscriptions,
            lifetime_subscriptions: raw.lifetime_subscriptions,
            views: raw.views,
            favorited: raw.favorited,
            time_created: raw.time_created,
            time_updated: raw.time_updated,
            file_size: raw.file_size,
            num_children: raw.num_children,
            children: raw.children,
            star_rating: raw.star_rating,
            total_votes: raw.total_votes,
            creator: raw.creator,
            file_type: raw.file_type,
        }
    }
}

/// The body of the SSR script element, or `None` when the page does not
/// carry one.
fn ssr_script_body(html: &str) -> Option<&str> {
    let attribute = html.find(SSR_ID_ATTRIBUTE)?;
    let body_start = html[attribute..].find('>')? + attribute + 1;
    let body_end = html[body_start..].find(SCRIPT_CLOSE)? + body_start;
    Some(&html[body_start..body_end])
}

/// Parse a browse page into its ranked result list.
pub(crate) fn parse_browse_page(html: &str) -> Result<BrowsePage, BrowseParseError> {
    let script = ssr_script_body(html).ok_or(BrowseParseError::MissingSsrScript)?;
    let envelope: SsrEnvelope = serde_json::from_str(script)
        .map_err(|error| BrowseParseError::BadScriptJson(error.to_string()))?;
    let query_data = envelope
        .render_context
        .and_then(|context| context.query_data)
        .ok_or_else(|| {
            BrowseParseError::QueryDataIsNotJson("renderContext.queryData is missing".to_string())
        })?;
    let parsed: QueryData = serde_json::from_str(&query_data)
        .map_err(|error| BrowseParseError::QueryDataIsNotJson(error.to_string()))?;

    let browse_state = parsed
        .queries
        .into_iter()
        .find(|query| {
            query.query_key.first().and_then(serde_json::Value::as_str) == Some(BROWSE_QUERY_KEY)
        })
        .and_then(|query| query.state)
        .and_then(|state| state.data)
        .ok_or(BrowseParseError::NoBrowseQuery)?;
    let browse: BrowseData = serde_json::from_value(browse_state).map_err(|error| {
        BrowseParseError::QueryDataIsNotJson(format!("browse state.data: {error}"))
    })?;

    Ok(BrowsePage {
        current_page: browse.current_page,
        total_pages: browse.total_pages,
        total_count: browse.total_count,
        results: browse.results.into_iter().map(BrowseResult::from).collect(),
    })
}

/// Browse URL for one query and page. `browsesort=textsearch` is
/// required: the default sort is `trend`, which does not rank by the
/// search text.
pub(crate) fn browse_url(query: &str, page: u32) -> String {
    format!(
        "{BROWSE_URL}?appid=107410&browsesort=textsearch&section=readytouseitems&requiredtags%5B%5D=Mod&searchtext={}&p={page}",
        crate::util::urlencode(query)
    )
}

/// Fetch and parse one page of a text search.
pub(crate) fn search_browse_page(
    http: &dyn Http,
    limiter: &mut RateLimiter,
    query: &str,
    page: u32,
) -> Result<BrowsePage, SearchError> {
    let url = browse_url(query, page);
    let html = fetch_with_retry(http, limiter, Endpoint::Browse, &url)?;
    Ok(parse_browse_page(&html)?)
}

/// Fetch and parse page one of a text search.
pub(crate) fn search_browse(
    http: &dyn Http,
    limiter: &mut RateLimiter,
    query: &str,
) -> Result<BrowsePage, SearchError> {
    search_browse_page(http, limiter, query, 1)
}

#[cfg(test)]
mod tests {
    use super::super::http::tests::FixtureHttp;
    use super::*;
    use std::time::Duration;

    const GPNVG18_PAGE: &str = include_str!("fixtures/browse_gpnvg18.html");
    const ADE_DIVING_PAGE: &str = include_str!("fixtures/browse_ade_diving.html");
    const NATO_MARKERS_PAGE: &str = include_str!("fixtures/browse_nato_markers.html");
    const EMPTY_PAGE: &str = include_str!("fixtures/browse_empty.html");
    const LEGACY_PAGE: &str = include_str!("fixtures/browse_legacy_ssr.html");
    const SIDEBAR_PAGE: &str = include_str!("fixtures/browse_sidebar_links.html");

    fn no_sleep(_: Duration) {}

    #[test]
    fn browse_parses_current_ssr_payload() {
        let page = parse_browse_page(GPNVG18_PAGE).unwrap();

        assert_eq!(page.results.len(), 30);
        assert_eq!(page.results[0].title, "L3-GPNVG18 Panoramic Night Vision");
        assert!(page.total_count > 0);
        assert!(page.results.iter().all(|result| {
            !result.id.is_empty() && result.id.chars().all(|c| c.is_ascii_digit())
        }));
        assert!(page
            .results
            .iter()
            .any(|result| result.title == "Jason's GPNVG-18 Night Vision"));
    }

    #[test]
    fn browse_preserves_rank_order() {
        let page = parse_browse_page(GPNVG18_PAGE).unwrap();

        // Ids in the order the captured page served them. Any sort,
        // dedupe or re-order in the parser breaks this list.
        let served = [
            "313041182",
            "1714019280",
            "2264203544",
            "2510824724",
            "864565270",
            "2801270017",
            "3320390980",
            "1849091860",
            "743099837",
            "1227179728",
            "3124431061",
            "3188912700",
            "2726974424",
            "2519184262",
            "1552807725",
            "498364936",
            "3308808194",
            "325853043",
            "2602680895",
            "2131302796",
            "3533685172",
            "3109927615",
            "2625657316",
            "1190645944",
            "3662188860",
            "1483246330",
            "1600297734",
            "2706945395",
            "3287007031",
            "1443024282",
        ];
        let parsed: Vec<String> = page
            .results
            .iter()
            .map(|result| result.id.clone())
            .collect();

        assert_eq!(parsed, served);
    }

    #[test]
    fn browse_rejects_legacy_render_context() {
        assert_eq!(
            parse_browse_page(LEGACY_PAGE),
            Err(BrowseParseError::MissingSsrScript)
        );
    }

    #[test]
    fn browse_rejects_page_without_ssr_even_when_links_exist() {
        assert_eq!(SIDEBAR_PAGE.matches("filedetails/?id=").count(), 5);

        assert_eq!(
            parse_browse_page(SIDEBAR_PAGE),
            Err(BrowseParseError::MissingSsrScript)
        );
    }

    #[test]
    fn browse_parses_empty_result_set() {
        let page = parse_browse_page(EMPTY_PAGE).unwrap();

        assert!(page.results.is_empty());
        assert_eq!(page.total_count, 0);
    }

    #[test]
    fn browse_url_uses_textsearch_sort_and_page() {
        let url = browse_url("ADE diving", 3);

        for fragment in [
            "appid=107410",
            "browsesort=textsearch",
            "section=readytouseitems",
            "requiredtags%5B%5D=Mod",
            "searchtext=ADE%20diving",
            "p=3",
        ] {
            assert!(url.contains(fragment), "missing {fragment} in {url}");
        }
    }

    #[test]
    fn browse_survives_missing_total_count() {
        let query_data = serde_json::json!({
            "queries": [{
                "queryKey": ["workshop_browse", {"appid": 107410}, 1],
                "state": {"data": {"current_page": 1, "results": [{"publishedfileid": "1", "title": "One"}]}},
            }],
        });
        let outer = serde_json::json!({"renderContext": {"queryData": query_data.to_string()}});
        let html = format!(
            "<html><body><script type=\"application/json\" id=\"valve-ssr-data\">{outer}</script></body></html>"
        );

        let page = parse_browse_page(&html).unwrap();

        assert_eq!(page.total_count, 0);
        assert_eq!(page.results.len(), 1);
    }

    #[test]
    fn search_browse_uses_injected_http() {
        let http = FixtureHttp::new(vec![Ok(GPNVG18_PAGE.to_string())]);
        let mut limiter = RateLimiter::new(no_sleep);

        let page = search_browse(&http, &mut limiter, "GPNVG18").unwrap();

        assert_eq!(page.results.len(), 30);
        assert_eq!(http.calls(), vec![browse_url("GPNVG18", 1)],);
    }

    #[test]
    fn search_browse_page_requests_page_two() {
        let http = FixtureHttp::new(vec![Ok(GPNVG18_PAGE.to_string())]);
        let mut limiter = RateLimiter::new(no_sleep);

        let page = search_browse_page(&http, &mut limiter, "GPNVG18", 2).unwrap();

        assert_eq!(page.results.len(), 30);
        let calls = http.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains("p=2"), "missing p=2 in {}", calls[0]);
    }

    #[test]
    fn browse_parses_other_live_pages() {
        let ade = parse_browse_page(ADE_DIVING_PAGE).unwrap();
        let nato = parse_browse_page(NATO_MARKERS_PAGE).unwrap();

        assert_eq!(
            ade.results[0].title,
            "ADE - Advanced Diving Environment (Working Keys | 2020)"
        );
        assert_eq!(nato.results[0].title, "NATO Markers+");
        assert_eq!(ade.results.len(), 30);
        assert_eq!(nato.results.len(), 30);
    }

    #[test]
    fn browse_preserves_file_type() {
        let query_data = serde_json::json!({
            "queries": [{
                "queryKey": ["workshop_browse", {}, 1],
                "state": {"data": {"total_count": 2, "results": [
                    {"publishedfileid": "1", "title": "A Mod", "file_type": 0},
                    {"publishedfileid": "2", "title": "A Collection", "file_type": 2},
                ]}},
            }],
        });
        let outer = serde_json::json!({"renderContext": {"queryData": query_data.to_string()}});
        let html = format!(
            "<html><body><script type=\"application/json\" id=\"valve-ssr-data\">{outer}</script></body></html>"
        );

        let page = parse_browse_page(&html).unwrap();

        assert_eq!(page.results[0].file_type, 0);
        assert_eq!(page.results[1].file_type, 2);
    }
}
