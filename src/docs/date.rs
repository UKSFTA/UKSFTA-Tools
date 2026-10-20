//! UTC date formatting without a date library. A changelog heading needs
//! only the year, month, and day, so one civil-from-days conversion is
//! enough.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds in one day.
const SECS_PER_DAY: i64 = 86_400;

/// Convert days since 1970-01-01 to a proleptic Gregorian date.
/// Algorithm from Howard Hinnant, "chrono-Compatible Low-Level Date
/// Algorithms" (days_from_civil, inverted).
pub const fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

/// Format a civil date as `YYYY-MM-DD`.
pub fn format_date(year: i64, month: u32, day: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}")
}

/// Format a Unix timestamp as a UTC `YYYY-MM-DD` date.
pub fn date_from_unix_secs(secs: i64) -> String {
    let days = secs.div_euclid(SECS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    format_date(year, month, day)
}

/// Today's date in UTC as `YYYY-MM-DD`. A clock before the Unix epoch
/// yields 1970-01-01, so the caller always gets a usable heading.
pub fn today_utc() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default();
    date_from_unix_secs(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_starts_at_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_handles_leap_day() {
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[test]
    fn civil_from_days_handles_new_year() {
        assert_eq!(civil_from_days(10_957), (2000, 1, 1));
        assert_eq!(civil_from_days(20_746), (2026, 10, 20));
    }

    #[test]
    fn civil_from_days_handles_day_before_epoch() {
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn format_date_pads_month_and_day() {
        assert_eq!(format_date(2026, 1, 2), "2026-01-02");
        assert_eq!(format_date(999, 12, 31), "0999-12-31");
    }

    #[test]
    fn date_from_unix_secs_converts_known_timestamps() {
        assert_eq!(date_from_unix_secs(0), "1970-01-01");
        assert_eq!(date_from_unix_secs(946_684_800), "2000-01-01");
        assert_eq!(date_from_unix_secs(1_709_164_800), "2024-02-29");
    }

    #[test]
    fn today_utc_has_date_shape() {
        let today = today_utc();
        assert_eq!(today.len(), 10);
        assert_eq!(today.as_bytes()[4], b'-');
        assert_eq!(today.as_bytes()[7], b'-');
    }
}
