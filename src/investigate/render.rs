//! The only place a resolution becomes text.
//!
//! Both `--online` and `--report` call this, so the two commands agree
//! byte for byte. A Workshop title appears only on a confirmed line.

use super::r#match::ResolutionStatus;
use super::resolve::PboResolution;
use crate::util::{paint, sanitize_for_terminal, C_GREEN, C_RED, C_YELLOW};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SummaryCounts {
    pub confirmed: usize,
    pub ambiguous: usize,
    pub no_query: usize,
    pub no_result: usize,
    pub rate_limited: usize,
    pub unresolved: usize,
}

pub(crate) fn summary_of(resolutions: &[PboResolution]) -> SummaryCounts {
    let mut counts = SummaryCounts::default();
    for item in resolutions {
        match item.resolution.status {
            ResolutionStatus::Confirmed => counts.confirmed += 1,
            ResolutionStatus::Ambiguous => counts.ambiguous += 1,
            ResolutionStatus::NoDistinctiveQuery => counts.no_query += 1,
            ResolutionStatus::NoResult => counts.no_result += 1,
            ResolutionStatus::RateLimited => counts.rate_limited += 1,
            ResolutionStatus::Unresolved => counts.unresolved += 1,
        }
    }
    counts
}

pub(crate) fn render_resolution(item: &PboResolution, use_colour: bool) -> String {
    let pbo = sanitize_for_terminal(&item.pbo);
    let resolution = &item.resolution;

    if resolution.status == ResolutionStatus::Confirmed {
        let title = sanitize_for_terminal(resolution.title.as_deref().unwrap_or_default());
        let id = sanitize_for_terminal(resolution.id.as_deref().unwrap_or_default());
        let query = sanitize_for_terminal(resolution.query.as_deref().unwrap_or_default());
        return format!(
            "{} {} -> \"{}\" ({}) [query \"{}\", M={:.2} Q={:.2} margin={:.2}]",
            paint(use_colour, C_GREEN, "[confirmed]"),
            pbo,
            title,
            id,
            query,
            resolution.m,
            resolution.q,
            resolution.margin
        );
    }

    let colour = match resolution.status {
        ResolutionStatus::Ambiguous => C_YELLOW,
        _ => C_RED,
    };
    let query = resolution
        .query
        .as_deref()
        .map(sanitize_for_terminal)
        .map(|text| format!("query \"{text}\""))
        .unwrap_or_else(|| "no query".to_string());
    let shared = resolution
        .matched_by
        .iter()
        .find_map(|signal| signal.strip_prefix("shared_title:"))
        .map(|title| format!(", shared title \"{}\"", sanitize_for_terminal(title)))
        .unwrap_or_default();
    format!(
        "{} {} - {} [{}, attempts={}, best M={:.2} Q={:.2}{}]",
        paint(
            use_colour,
            colour,
            &format!("[{}]", resolution.status.label())
        ),
        pbo,
        status_text(resolution.status),
        query,
        item.attempts.len(),
        resolution.m,
        resolution.q,
        shared
    )
}

/// Every resolution as one block of lines. Both `--online` and `--report`
/// print this, so the two paths cannot drift.
pub(crate) fn render_all(resolutions: &[PboResolution], use_colour: bool) -> String {
    resolutions
        .iter()
        .map(|item| render_resolution(item, use_colour))
        .collect::<Vec<_>>()
        .join("\n")
}

fn status_text(status: ResolutionStatus) -> &'static str {
    match status {
        ResolutionStatus::Confirmed => "confirmed",
        ResolutionStatus::Ambiguous => "more than one candidate",
        ResolutionStatus::NoDistinctiveQuery => "no distinctive identity",
        ResolutionStatus::NoResult => "no candidates",
        ResolutionStatus::RateLimited => "rate limited, retry later",
        ResolutionStatus::Unresolved => "unresolved",
    }
}

pub(crate) fn render_summary(counts: &SummaryCounts, use_colour: bool) -> String {
    format!(
        "{}\n  {} confirmed\n  {} ambiguous\n  {} no-query\n  {} no-result\n  {} rate-limited\n  {} unresolved",
        paint(use_colour, C_GREEN, "Summary:"),
        counts.confirmed,
        counts.ambiguous,
        counts.no_query,
        counts.no_result,
        counts.rate_limited,
        counts.unresolved
    )
}

/// An elapsed time as `MM:SS`, or `H:MM:SS` from one hour.
pub(crate) fn format_elapsed(secs: u64) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    if hours == 0 {
        format!("{minutes:02}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
}

/// Longest progress label kept whole. A longer one is cut and marked.
const PROGRESS_LABEL_MAX: usize = 40;

/// One progress line for a paced scan, with no newline and no colour, so a
/// caller can rewrite the line in place.
pub(crate) fn render_progress(
    done: usize,
    total: usize,
    elapsed: std::time::Duration,
    label: &str,
) -> String {
    let width = total.to_string().len();
    let percent = done.saturating_mul(100).checked_div(total).unwrap_or(0);
    let elapsed_secs = elapsed.as_secs();
    let eta = if done == 0 || elapsed_secs == 0 {
        "--:--".to_string()
    } else {
        let remaining =
            elapsed_secs.saturating_mul(total.saturating_sub(done) as u64) / done as u64;
        format_elapsed(remaining)
    };
    let mut line = format!(
        "  [ {done:>width$}/ {total:>width$} {percent:>3}%] {} elapsed  ETA {eta}",
        format_elapsed(elapsed_secs)
    );
    if !label.is_empty() {
        let mut shown: String = label.chars().take(PROGRESS_LABEL_MAX).collect();
        if label.chars().count() > PROGRESS_LABEL_MAX {
            shown.push_str("...");
        }
        line.push_str("  ");
        line.push_str(&shown);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::investigate::identity::IdentityEvidence;
    use crate::investigate::r#match::Resolution;
    use std::time::Duration;

    fn item(status: ResolutionStatus, title: Option<&str>) -> PboResolution {
        PboResolution {
            pbo: "ade_core.pbo".to_string(),
            fingerprint: "fp".to_string(),
            evidence: IdentityEvidence {
                pbo: "ade_core.pbo".to_string(),
                ..Default::default()
            },
            attempts: Vec::new(),
            resolution: Resolution {
                status,
                id: title.map(|_| "1".to_string()),
                title: title.map(str::to_string),
                m: 1.0,
                q: 0.8,
                c: 0.8,
                margin: 1.0,
                query: Some("ADE diving".to_string()),
                matched_by: vec!["title".to_string()],
            },
            candidates: Vec::new(),
        }
    }

    #[test]
    fn render_confirmed_line_has_arrow_and_title() {
        let line = render_resolution(
            &item(
                ResolutionStatus::Confirmed,
                Some("ADE - Advanced Diving Environment"),
            ),
            false,
        );

        assert!(line.starts_with("[confirmed] "));
        assert!(line.contains(" -> \"ADE - Advanced Diving Environment\" (1)"));
        assert!(line.contains("query \"ADE diving\""));
    }

    #[test]
    fn render_unconfirmed_line_has_no_title() {
        let line = render_resolution(&item(ResolutionStatus::Ambiguous, None), false);

        assert!(line.starts_with("[ambiguous] "));
        assert!(!line.contains(" -> "));
        assert!(!line.contains("ADE - Advanced Diving Environment"));
    }

    #[test]
    fn render_sanitises_control_characters() {
        let line = render_resolution(
            &item(
                ResolutionStatus::Confirmed,
                Some("\u{1b}[31mSpoofed\u{1b}[0m"),
            ),
            false,
        );

        assert!(!line.contains('\u{1b}'));
        assert!(line.contains("Spoofed"));
    }

    #[test]
    fn summary_counts_match_statuses() {
        let items = vec![
            item(ResolutionStatus::Confirmed, Some("A")),
            item(ResolutionStatus::Confirmed, Some("B")),
            item(ResolutionStatus::Ambiguous, None),
            item(ResolutionStatus::NoDistinctiveQuery, None),
            item(ResolutionStatus::NoResult, None),
            item(ResolutionStatus::RateLimited, None),
        ];

        let counts = summary_of(&items);

        assert_eq!(counts.confirmed, 2);
        assert_eq!(counts.ambiguous, 1);
        assert_eq!(counts.no_query, 1);
        assert_eq!(counts.no_result, 1);
        assert_eq!(counts.rate_limited, 1);
        assert_eq!(counts.unresolved, 0);
    }

    #[test]
    fn format_elapsed_renders_minutes_and_seconds() {
        assert_eq!(format_elapsed(0), "00:00");
        assert_eq!(format_elapsed(92), "01:32");
        assert_eq!(format_elapsed(3599), "59:59");
    }

    #[test]
    fn format_elapsed_renders_hours() {
        assert_eq!(format_elapsed(3600), "1:00:00");
        assert_eq!(format_elapsed(3672), "1:01:12");
    }

    #[test]
    fn render_progress_aligns_fields_and_reports_eta() {
        let line = render_progress(12, 57, Duration::from_secs(92), "esp_objects");

        assert_eq!(
            line,
            "  [ 12/ 57  21%] 01:32 elapsed  ETA 05:45  esp_objects"
        );
    }

    #[test]
    fn render_progress_first_step_shows_no_eta() {
        let line = render_progress(0, 57, Duration::from_secs(0), "esp_objects");

        assert_eq!(
            line,
            "  [  0/ 57   0%] 00:00 elapsed  ETA --:--  esp_objects"
        );
    }

    #[test]
    fn render_progress_zero_elapsed_shows_no_eta() {
        let line = render_progress(1, 57, Duration::from_secs(0), "esp_objects");

        assert_eq!(
            line,
            "  [  1/ 57   1%] 00:00 elapsed  ETA --:--  esp_objects"
        );
    }

    #[test]
    fn render_progress_complete_shows_full_percent_and_zero_eta() {
        let line = render_progress(57, 57, Duration::from_secs(92), "");

        assert_eq!(line, "  [ 57/ 57 100%] 01:32 elapsed  ETA 00:00");
        assert!(!line.ends_with(' '));
    }

    #[test]
    fn render_progress_truncates_long_label() {
        let label = "a".repeat(50);

        let line = render_progress(1, 2, Duration::from_secs(1), &label);

        assert!(line.ends_with(&format!("{}{}", "a".repeat(40), "...")));
    }

    #[test]
    fn render_progress_leaves_no_trailing_space_without_label() {
        let line = render_progress(1, 2, Duration::from_secs(1), "");

        assert_eq!(line, "  [ 1/ 2  50%] 00:01 elapsed  ETA 00:01");
        assert!(!line.ends_with(' '));
    }

    #[test]
    fn render_progress_treats_empty_total_as_zero_percent() {
        let line = render_progress(0, 0, Duration::from_secs(0), "");

        assert_eq!(line, "  [ 0/ 0   0%] 00:00 elapsed  ETA --:--");
    }
}

#[cfg(test)]
mod shared_render_tests {
    use super::*;
    use crate::investigate::identity::IdentityEvidence;
    use crate::investigate::r#match::Resolution;

    #[test]
    fn ambiguous_line_reports_shared_title() {
        let item = PboResolution {
            pbo: "ade_core.pbo".to_string(),
            fingerprint: "fp".to_string(),
            evidence: IdentityEvidence::default(),
            attempts: Vec::new(),
            candidates: Vec::new(),
            resolution: Resolution {
                status: ResolutionStatus::Ambiguous,
                id: None,
                title: None,
                m: 1.0,
                q: 0.88,
                c: 0.88,
                margin: 0.0,
                query: Some("Advanced Diving Environment".to_string()),
                matched_by: vec!["shared_title:Advanced Diving Environment".to_string()],
            },
        };

        let line = render_resolution(&item, false);

        assert!(line.contains("shared title \"Advanced Diving Environment\""));
        assert!(!line.contains(" -> "));
        assert!(line.starts_with("[ambiguous] "));
    }
}
