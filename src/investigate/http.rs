//! Injectable HTTP seam with one pacing interval per Steam endpoint.
//!
//! Every network call in the investigate path goes through [`Http`], so a
//! test drives the caller with a recorded page and never opens a socket.
//! The interval table and the pure [`required_wait`] helper are the only
//! place a delay is decided.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The Steam endpoints the investigate path may call. The declaration
/// order is the index into the interval and last-call tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Endpoint {
    Browse,
    SharedFiles,
}

impl Endpoint {
    fn index(self) -> usize {
        match self {
            Endpoint::Browse => 0,
            Endpoint::SharedFiles => 1,
        }
    }
}

/// Minimum seconds between two calls to the same endpoint. Browse is a
/// full HTML page and sharedfiles is a detail page.
const ENDPOINT_INTERVALS: [Duration; 2] = [Duration::from_secs(4), Duration::from_secs(8)];

/// The first backoff a rate-limited response waits, plus up to
/// [`BACKOFF_JITTER_MS`]. Each further retry doubles it.
const BACKOFF_BASE: Duration = Duration::from_secs(30);
const BACKOFF_JITTER_MS: u32 = 5_000;

/// The longest `Retry-After` a 429 may impose before a retry.
const RETRY_AFTER_CAP_SECS: u64 = 300;

/// A failed request. `Transport` keeps the message as text so no
/// `reqwest` type reaches a caller or a test signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HttpError {
    Transport(String),
    RateLimited { retry_after: Option<u64> },
    Status(u16),
}

impl HttpError {
    pub(crate) fn is_rate_limit(&self) -> bool {
        matches!(self, HttpError::RateLimited { .. } | HttpError::Status(429))
    }

    /// A transient server fault. It gets a short retry, never the long
    /// throttle backoff, so one 503 cannot stall a scan.
    pub(crate) fn is_server_error(&self) -> bool {
        matches!(self, HttpError::Status(503))
    }
}

/// One GET, returning the response body as text.
pub(crate) trait Http {
    fn get(&self, url: &str) -> Result<String, HttpError>;
}

/// The live client, built from the shared Workshop client policy.
pub(crate) struct LiveHttp {
    client: reqwest::blocking::Client,
}

impl LiveHttp {
    pub(crate) fn new() -> Self {
        Self {
            client: crate::workshop_api::build_client(),
        }
    }
}

impl Http for LiveHttp {
    fn get(&self, url: &str) -> Result<String, HttpError> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|error| HttpError::Transport(error.to_string()))?;
        let status = response.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(|secs| secs.min(RETRY_AFTER_CAP_SECS));
            return Err(HttpError::RateLimited { retry_after });
        }
        if !status.is_success() {
            return Err(HttpError::Status(status.as_u16()));
        }
        response
            .text()
            .map_err(|error| HttpError::Transport(error.to_string()))
    }
}

/// Per-endpoint pacing plus the consecutive rate-limit counter.
///
/// `sleep` is injected so a test records delays instead of waiting.
pub(crate) struct RateLimiter {
    last: [Option<Instant>; 2],
    sleep: fn(Duration),
}

impl RateLimiter {
    pub(crate) fn new(sleep: fn(Duration)) -> Self {
        Self {
            last: [None; 2],
            sleep,
        }
    }

    /// Sleep for the remainder of the endpoint interval, then record the
    /// call time.
    pub(crate) fn wait(&mut self, endpoint: Endpoint) {
        let index = endpoint.index();
        let now = Instant::now();
        if let Some(wait) = required_wait(self.last[index], now, ENDPOINT_INTERVALS[index]) {
            (self.sleep)(wait);
        }
        self.last[index] = Some(now);
    }

    /// Sleep for one backoff step. The function is injected, so a test
    /// records the delay instead of waiting.
    fn pause(&mut self, wait: Duration) {
        (self.sleep)(wait);
    }
}

/// How long the nth rate-limit retry waits, jitter included.
fn backoff_wait(attempt: u32) -> Duration {
    let factor = 1u32 << attempt.min(3);
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos() % BACKOFF_JITTER_MS)
        .unwrap_or(0);
    BACKOFF_BASE * factor + Duration::from_millis(u64::from(jitter))
}

/// How long the nth server-fault retry waits: five seconds, then fifteen.
fn server_error_wait(attempt: u32) -> Duration {
    SERVER_ERROR_WAIT * 3u32.pow(attempt)
}

/// How long to wait before the next call, given the previous call time.
/// `None` means the interval has already elapsed.
pub(crate) fn required_wait(
    last: Option<Instant>,
    now: Instant,
    interval: Duration,
) -> Option<Duration> {
    let last = last?;
    let elapsed = now.saturating_duration_since(last);
    (elapsed < interval).then(|| interval - elapsed)
}

/// How many times a rate-limited request waits and tries again before the
/// run records that PBO as rate-limited.
const MAX_RATE_LIMIT_RETRIES: u32 = 5;

/// How many times a 503 is retried before the status is returned.
const MAX_SERVER_ERROR_RETRIES: u32 = 2;

/// The wait before the first 503 retry. The second waits three times as
/// long, so a transient fault costs twenty seconds, not eleven minutes.
const SERVER_ERROR_WAIT: Duration = Duration::from_secs(5);

/// Fetch `url` under the endpoint interval. A rate-limited response waits
/// out an exponential backoff and tries again, up to
/// [`MAX_RATE_LIMIT_RETRIES`], so a long scan continues after a throttle
/// instead of stopping at the first one. A 503 is a different fault: it
/// gets two short retries and then the status, so one bad minute on the
/// server does not hold up the whole pack.
pub(crate) fn fetch_with_retry(
    http: &dyn Http,
    limiter: &mut RateLimiter,
    endpoint: Endpoint,
    url: &str,
) -> Result<String, HttpError> {
    // The first call respects the endpoint interval. Every retry follows a
    // backoff longer than that interval, so it needs no second wait.
    limiter.wait(endpoint);
    let mut rate_limit_retries = 0;
    let mut server_error_retries = 0;
    loop {
        match http.get(url) {
            Ok(body) => return Ok(body),
            Err(error) if error.is_rate_limit() => {
                if rate_limit_retries == MAX_RATE_LIMIT_RETRIES {
                    return Err(HttpError::RateLimited { retry_after: None });
                }
                let backoff = backoff_wait(rate_limit_retries);
                let wait = match error {
                    HttpError::RateLimited {
                        retry_after: Some(secs),
                    } => backoff.max(Duration::from_secs(secs)),
                    _ => backoff,
                };
                limiter.pause(wait);
                rate_limit_retries += 1;
            }
            Err(error) if error.is_server_error() => {
                if server_error_retries == MAX_SERVER_ERROR_RETRIES {
                    return Err(error);
                }
                limiter.pause(server_error_wait(server_error_retries));
                server_error_retries += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// Replays canned responses and records every URL requested.
    pub(crate) struct FixtureHttp {
        responses: RefCell<VecDeque<Result<String, HttpError>>>,
        calls: RefCell<Vec<String>>,
    }

    impl FixtureHttp {
        pub(crate) fn new(responses: Vec<Result<String, HttpError>>) -> Self {
            Self {
                responses: RefCell::new(responses.into()),
                calls: RefCell::new(Vec::new()),
            }
        }

        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl Http for FixtureHttp {
        fn get(&self, url: &str) -> Result<String, HttpError> {
            self.calls.borrow_mut().push(url.to_string());
            self.responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err(HttpError::Transport("no canned response".to_string())))
        }
    }

    thread_local! {
        static SLEEPS: RefCell<Vec<Duration>> = const { RefCell::new(Vec::new()) };
    }

    fn record_sleep(wait: Duration) {
        SLEEPS.with(|sleeps| sleeps.borrow_mut().push(wait));
    }

    fn take_sleeps() -> Vec<Duration> {
        SLEEPS.with(|sleeps| std::mem::take(&mut *sleeps.borrow_mut()))
    }

    fn ok(body: &str) -> Result<String, HttpError> {
        Ok(body.to_string())
    }

    fn rate_limited() -> Result<String, HttpError> {
        Err(HttpError::Status(429))
    }

    #[test]
    fn required_wait_first_call_is_none() {
        assert_eq!(
            required_wait(None, Instant::now(), Duration::from_secs(4)),
            None
        );
    }

    #[test]
    fn required_wait_returns_remainder() {
        let now = Instant::now();
        let last = now - Duration::from_millis(1_500);
        assert_eq!(
            required_wait(Some(last), now, Duration::from_secs(4)),
            Some(Duration::from_millis(2_500))
        );
    }

    #[test]
    fn required_wait_returns_none_after_interval() {
        let now = Instant::now();
        let last = now - Duration::from_secs(9);
        assert_eq!(required_wait(Some(last), now, Duration::from_secs(8)), None);
    }

    #[test]
    fn http_retries_once_after_rate_limit() {
        take_sleeps();
        let http = FixtureHttp::new(vec![rate_limited(), ok("body")]);
        let mut limiter = RateLimiter::new(record_sleep);

        let result = fetch_with_retry(&http, &mut limiter, Endpoint::Browse, "https://example/1");

        assert_eq!(result, Ok("body".to_string()));
        assert_eq!(http.calls().len(), 2);
        let sleeps = take_sleeps();
        assert_eq!(sleeps.len(), 1);
        assert!(sleeps[0] >= Duration::from_secs(30));
        assert!(sleeps[0] <= Duration::from_secs(35));
    }

    #[test]
    fn http_retries_through_repeated_rate_limits() {
        take_sleeps();
        let http = FixtureHttp::new(vec![
            rate_limited(),
            rate_limited(),
            rate_limited(),
            ok("body"),
        ]);
        let mut limiter = RateLimiter::new(record_sleep);

        let result = fetch_with_retry(&http, &mut limiter, Endpoint::Browse, "https://example/2");

        assert_eq!(result, Ok("body".to_string()));
        assert_eq!(http.calls().len(), 4);
        let sleeps = take_sleeps();
        assert_eq!(sleeps.len(), 3);
        assert!(sleeps[0] >= Duration::from_secs(30));
        assert!(sleeps[1] >= Duration::from_secs(60));
        assert!(sleeps[2] >= Duration::from_secs(120));
    }

    #[test]
    fn http_gives_up_after_five_rate_limit_retries() {
        take_sleeps();
        let http = FixtureHttp::new(vec![
            rate_limited(),
            rate_limited(),
            rate_limited(),
            rate_limited(),
            rate_limited(),
            rate_limited(),
        ]);
        let mut limiter = RateLimiter::new(record_sleep);

        let result = fetch_with_retry(&http, &mut limiter, Endpoint::Browse, "https://example/3");

        assert_eq!(result, Err(HttpError::RateLimited { retry_after: None }));
        assert_eq!(http.calls().len(), 6);
    }

    #[test]
    fn http_retries_a_server_error_twice_then_reports_it() {
        take_sleeps();
        let http = FixtureHttp::new(vec![
            Err(HttpError::Status(503)),
            Err(HttpError::Status(503)),
            Err(HttpError::Status(503)),
        ]);
        let mut limiter = RateLimiter::new(record_sleep);

        let result = fetch_with_retry(&http, &mut limiter, Endpoint::Browse, "https://example/7");

        assert_eq!(result, Err(HttpError::Status(503)));
        assert_eq!(http.calls().len(), 3);
        assert_eq!(
            take_sleeps(),
            [Duration::from_secs(5), Duration::from_secs(15)]
        );
        let error = result.unwrap_err();
        assert!(error.is_server_error());
        assert!(!error.is_rate_limit());
    }

    #[test]
    fn http_waits_for_the_retry_after_header() {
        take_sleeps();
        let http = FixtureHttp::new(vec![
            Err(HttpError::RateLimited {
                retry_after: Some(120),
            }),
            ok("body"),
        ]);
        let mut limiter = RateLimiter::new(record_sleep);

        let result = fetch_with_retry(&http, &mut limiter, Endpoint::Browse, "https://example/8");

        assert_eq!(result, Ok("body".to_string()));
        let sleeps = take_sleeps();
        assert_eq!(sleeps.len(), 1);
        assert!(sleeps[0] >= Duration::from_secs(120));
    }

    #[test]
    fn http_enforces_browse_interval() {
        take_sleeps();
        let http = FixtureHttp::new(vec![ok("a"), ok("b")]);
        let mut limiter = RateLimiter::new(record_sleep);

        fetch_with_retry(&http, &mut limiter, Endpoint::Browse, "https://example/3").unwrap();
        fetch_with_retry(&http, &mut limiter, Endpoint::Browse, "https://example/4").unwrap();

        let sleeps = take_sleeps();
        assert_eq!(sleeps.len(), 1);
        assert!(sleeps[0] > Duration::from_millis(3_900));
        assert!(sleeps[0] <= Duration::from_secs(4));
    }

    #[test]
    fn http_enforces_sharedfiles_interval() {
        take_sleeps();
        let http = FixtureHttp::new(vec![ok("a"), ok("b")]);
        let mut limiter = RateLimiter::new(record_sleep);

        fetch_with_retry(
            &http,
            &mut limiter,
            Endpoint::SharedFiles,
            "https://example/5",
        )
        .unwrap();
        fetch_with_retry(
            &http,
            &mut limiter,
            Endpoint::SharedFiles,
            "https://example/6",
        )
        .unwrap();

        let sleeps = take_sleeps();
        assert_eq!(sleeps.len(), 1);
        assert!(sleeps[0] > Duration::from_millis(7_900));
        assert!(sleeps[0] <= Duration::from_secs(8));
    }
}
