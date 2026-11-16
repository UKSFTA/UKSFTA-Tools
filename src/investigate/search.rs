//! Keyless Workshop search: a thin adapter over [`browse`].
//!
//! The browse page is the only keyless search path. A page without the
//! current SSR payload is an error, never a list of sidebar ids.

use super::browse::{self, BrowsePage, SearchError};
use super::http::{Http, RateLimiter};

/// Fetch page one of a text search, results in served rank order.
pub(crate) fn search(
    http: &dyn Http,
    limiter: &mut RateLimiter,
    query: &str,
) -> Result<BrowsePage, SearchError> {
    browse::search_browse(http, limiter, query)
}

#[cfg(test)]
mod tests {
    use super::super::browse::BrowseParseError;
    use super::super::http::tests::FixtureHttp;
    use super::*;
    use std::time::Duration;

    const GPNVG18_PAGE: &str = include_str!("fixtures/browse_gpnvg18.html");
    const SIDEBAR_PAGE: &str = include_str!("fixtures/browse_sidebar_links.html");

    fn no_sleep(_: Duration) {}

    #[test]
    fn search_returns_ranked_browse_page() {
        let http = FixtureHttp::new(vec![Ok(GPNVG18_PAGE.to_string())]);
        let mut limiter = RateLimiter::new(no_sleep);

        let page = search(&http, &mut limiter, "GPNVG18").unwrap();

        assert_eq!(page.results.len(), 30);
        assert_eq!(page.results[0].title, "L3-GPNVG18 Panoramic Night Vision");
        assert_eq!(http.calls().len(), 1);
    }

    #[test]
    fn search_rejects_page_without_ssr_payload() {
        let http = FixtureHttp::new(vec![Ok(SIDEBAR_PAGE.to_string())]);
        let mut limiter = RateLimiter::new(no_sleep);

        let result = search(&http, &mut limiter, "ADE diving");

        assert_eq!(
            result,
            Err(SearchError::Parse(BrowseParseError::MissingSsrScript))
        );
    }
}
