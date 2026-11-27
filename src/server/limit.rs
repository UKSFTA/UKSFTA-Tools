//! The upload rate: the `--limit` parser and the shared pacer that the
//! segmented connections use.

use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use crate::error::UksftaError;

/// Parse a byte rate. A suffix is 1024-based: `500K`, `2M`, `1.5M`, `1G`.
///
/// # Errors
/// Returns `Input` for a zero, negative, unknown-suffix or overflowing rate.
pub fn parse_rate(text: &str) -> Result<u64, UksftaError> {
    let text = text.trim();
    let invalid = || {
        UksftaError::Input(format!(
            "limit must be a positive rate such as 500K, 2M or 1.5M: {text:?}"
        ))
    };
    let (digits, factor) = match text.chars().last() {
        Some('K' | 'k') => (&text[..text.len() - 1], 1024_u64),
        Some('M' | 'm') => (&text[..text.len() - 1], 1024 * 1024),
        Some('G' | 'g') => (&text[..text.len() - 1], 1024 * 1024 * 1024),
        Some(c) if c.is_ascii_digit() || c == '.' => (text, 1),
        _ => return Err(invalid()),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(invalid());
    }
    let value: f64 = digits.parse().map_err(|_| invalid())?;
    if !value.is_finite() || value <= 0.0 {
        return Err(invalid());
    }
    let bytes = value * factor as f64;
    if bytes >= u64::MAX as f64 {
        return Err(UksftaError::Input(format!("limit is too large: {text:?}")));
    }
    let bytes = bytes.round() as u64;
    if bytes == 0 {
        return Err(invalid());
    }
    Ok(bytes)
}

/// A shared pacer for one rate. Each block reserves the next slot, so the
/// total across every connection stays at the rate.
pub struct RateLimiter {
    rate: u64,
    next: Mutex<Instant>,
}

impl RateLimiter {
    /// Pace at `bytes_per_second`. A zero rate is treated as one byte per
    /// second; the parser never produces one.
    pub fn new(bytes_per_second: u64) -> Self {
        Self {
            rate: bytes_per_second.max(1),
            next: Mutex::new(Instant::now()),
        }
    }

    /// Reserve the next slot. One block may go at once; later blocks wait.
    pub fn reserve(&self, bytes: u64) -> Instant {
        let mut next = self.next.lock().unwrap_or_else(|error| error.into_inner());
        let now = Instant::now();
        let slot = (*next).max(now);
        let seconds = bytes as f64 / self.rate as f64;
        *next = slot + Duration::from_secs_f64(seconds);
        slot
    }

    /// Wait for the next slot.
    pub async fn wait(&self, bytes: u64) {
        tokio::time::sleep_until(self.reserve(bytes)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_rate_is_bytes_per_second() {
        assert_eq!(parse_rate("1000").unwrap(), 1000);
        assert_eq!(parse_rate("512").unwrap(), 512);
    }

    #[test]
    fn a_suffix_is_1024_based() {
        assert_eq!(parse_rate("500K").unwrap(), 512_000);
        assert_eq!(parse_rate("2M").unwrap(), 2_097_152);
        assert_eq!(parse_rate("1.5M").unwrap(), 1_572_864);
        assert_eq!(parse_rate("1G").unwrap(), 1_073_741_824);
        assert_eq!(parse_rate("1m").unwrap(), 1_048_576);
    }

    #[test]
    fn a_bad_rate_is_refused() {
        for text in [
            "0", "0K", "-1M", "+1M", "5T", "", "M", "1.2.3", "abc", "1e3",
        ] {
            assert!(
                matches!(parse_rate(text), Err(UksftaError::Input(_))),
                "{text:?} must be refused"
            );
        }
    }

    #[test]
    fn an_overflow_is_refused() {
        assert!(matches!(
            parse_rate("99999999999999999999G"),
            Err(UksftaError::Input(_))
        ));
        assert!(matches!(
            parse_rate("18446744073709551616"),
            Err(UksftaError::Input(_))
        ));
    }

    #[test]
    fn the_second_block_waits_for_the_first_slot() {
        let limiter = RateLimiter::new(1_000_000);
        let first = limiter.reserve(100_000);
        let second = limiter.reserve(100_000);
        let gap = second.duration_since(first);
        assert!(
            gap >= Duration::from_millis(99) && gap <= Duration::from_millis(101),
            "{gap:?}"
        );
    }

    #[test]
    fn a_large_block_reserves_its_own_time() {
        let limiter = RateLimiter::new(1_000_000);
        let first = limiter.reserve(1_000_000);
        let second = limiter.reserve(1);
        assert!(second.duration_since(first) >= Duration::from_secs(1));
    }
}
