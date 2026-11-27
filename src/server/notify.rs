//! One desktop notification when a run finishes.

use crate::error::UksftaError;
use crate::util::{human_size, sanitize_for_terminal};

/// Environment override for the notifier binary. A test points this at a
/// stub that records its arguments.
const BIN_ENV: &str = "UKSFTA_NOTIFY_BIN";
/// The notification title.
const TITLE: &str = "uksfta";
/// The longest error text a notification carries.
const MAX_ERROR: usize = 200;

/// What finished, for the notification body.
pub struct Finished<'a> {
    pub pack: Option<(&'a str, &'a str)>,
    pub result: &'a Result<(), UksftaError>,
    pub files: usize,
    pub bytes: u64,
    pub seconds: f64,
}

/// The notifier this platform uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notifier {
    NotifySend,
    Osascript,
    /// A test binary from `UKSFTA_NOTIFY_BIN`.
    Override(String),
    /// No desktop notifier on this platform.
    None,
}

/// Pick the notifier. The override always wins.
pub fn select(overridden: Option<String>) -> Notifier {
    if let Some(binary) = overridden {
        return Notifier::Override(binary);
    }
    if cfg!(target_os = "linux") {
        Notifier::NotifySend
    } else if cfg!(target_os = "macos") {
        Notifier::Osascript
    } else {
        Notifier::None
    }
}

/// The notification body: the pack, the result, the file count, the bytes
/// and the elapsed seconds.
pub fn body(finished: &Finished<'_>) -> String {
    let pack = finished.pack.map_or_else(
        || "unknown pack".to_string(),
        |(id, name)| format!("pack {id} {name}"),
    );
    let outcome = match finished.result {
        Ok(()) => "ok".to_string(),
        Err(error) => format!("failed: {}", short_error(error)),
    };
    format!(
        "{pack}: {outcome}, {} file(s), {}, {:.1} s",
        finished.files,
        human_size(finished.bytes),
        finished.seconds
    )
}

/// A sanitised error, short enough for a notification.
fn short_error(error: &UksftaError) -> String {
    let text = sanitize_for_terminal(&error.to_string());
    let text = text.trim();
    if text.chars().count() > MAX_ERROR {
        let mut short: String = text.chars().take(MAX_ERROR).collect();
        short.push_str("...");
        short
    } else {
        text.to_string()
    }
}

/// Send the notification. The exit status is ignored, so a missing desktop
/// session never fails the run.
pub fn send(finished: &Finished<'_>) {
    let text = body(finished);
    match select(std::env::var(BIN_ENV).ok()) {
        Notifier::Override(binary) => run(&binary, &[TITLE.to_string(), text]),
        Notifier::NotifySend => run("notify-send", &[TITLE.to_string(), text]),
        Notifier::Osascript => {
            let script = format!(
                "display notification \"{}\" with title \"{TITLE}\"",
                escape_applescript(&text)
            );
            run("osascript", &["-e".to_string(), script]);
        }
        Notifier::None => eprintln!("Warning: no desktop notifier on this platform"),
    }
}

fn run(binary: &str, args: &[String]) {
    if let Err(error) = std::process::Command::new(binary).args(args).status() {
        eprintln!("Warning: cannot send the notification: {error}");
    }
}

/// Quote for an AppleScript string literal.
fn escape_applescript(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Result<(), UksftaError> {
        Ok(())
    }

    fn failed() -> Result<(), UksftaError> {
        Err(UksftaError::Input("the server refused the key".to_string()))
    }

    #[test]
    fn the_body_names_the_pack_the_result_and_the_bytes() {
        let result = ok();
        let text = body(&Finished {
            pack: Some(("999999997", "Obs")),
            result: &result,
            files: 2,
            bytes: 8_388_608,
            seconds: 2.2,
        });
        assert_eq!(text, "pack 999999997 Obs: ok, 2 file(s), 8.4 MB, 2.2 s");
    }

    #[test]
    fn a_failure_body_carries_the_error() {
        let result = failed();
        let text = body(&Finished {
            pack: None,
            result: &result,
            files: 0,
            bytes: 0,
            seconds: 0.1,
        });
        assert!(
            text.starts_with("unknown pack: failed: the server refused the key"),
            "{text}"
        );
        assert!(text.contains("0 file(s)"), "{text}");
        assert!(text.contains("0 B"), "{text}");
    }

    #[test]
    fn a_long_error_is_cut() {
        let result: Result<(), UksftaError> = Err(UksftaError::Input("x".repeat(MAX_ERROR + 50)));
        let text = body(&Finished {
            pack: Some(("1", "P")),
            result: &result,
            files: 0,
            bytes: 0,
            seconds: 0.0,
        });
        assert!(text.contains("..."), "{text}");
        assert!(text.chars().count() < MAX_ERROR + 60, "{text}");
    }

    #[test]
    fn the_platform_selection_matches_the_build_target() {
        let selected = select(None);
        if cfg!(target_os = "linux") {
            assert_eq!(selected, Notifier::NotifySend);
        } else if cfg!(target_os = "macos") {
            assert_eq!(selected, Notifier::Osascript);
        } else {
            assert_eq!(selected, Notifier::None);
        }
    }

    #[test]
    fn the_override_wins() {
        assert_eq!(
            select(Some("/tmp/stub".to_string())),
            Notifier::Override("/tmp/stub".to_string())
        );
    }

    #[test]
    fn an_applescript_body_escapes_quotes_and_backslashes() {
        assert_eq!(escape_applescript("a \"b\" \\c"), "a \\\"b\\\" \\\\c");
    }
}
