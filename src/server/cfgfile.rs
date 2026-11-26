//! Read the three server settings that decide signature enforcement.
//!
//! The scanner ignores comments and string contents, so a `hostname` that
//! holds `verifySignatures = 0;` cannot be mistaken for the setting. It
//! never evaluates an expression and never follows an include.

/// The largest config read.
const MAX_BYTES: usize = 1024 * 1024;

/// The statements at brace depth 0.
#[derive(Debug, Clone, Default)]
pub struct Cfg {
    entries: Vec<(String, String)>,
}

/// One result of the security checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CfgFinding {
    /// The setting value, or None when the key is absent.
    VerifySignatures(Option<String>),
    AllowedFilePatching(String),
    FilePatchingExceptions,
    EqualModRequired(String),
}

impl CfgFinding {
    /// The report line, without the `server.cfg: ` prefix.
    pub fn describe(&self) -> String {
        match self {
            CfgFinding::VerifySignatures(Some(value)) => format!("verifySignatures = {value}"),
            CfgFinding::VerifySignatures(None) => {
                "verifySignatures is missing (the engine default is 0)".to_string()
            }
            CfgFinding::AllowedFilePatching(value) => format!("allowedFilePatching = {value}"),
            CfgFinding::FilePatchingExceptions => "filePatchingExceptions is not empty".to_string(),
            CfgFinding::EqualModRequired(value) => {
                format!("equalModRequired = {value} (advisory)")
            }
        }
    }

    /// True when the value fails a required check.
    pub fn fatal(&self) -> bool {
        match self {
            CfgFinding::VerifySignatures(Some(value)) => value.trim() != "2",
            CfgFinding::VerifySignatures(None) => true,
            CfgFinding::AllowedFilePatching(value) => value.trim() != "0",
            CfgFinding::FilePatchingExceptions | CfgFinding::EqualModRequired(_) => false,
        }
    }
}

/// Parse the text. Only brace-depth-0 statements are kept, and the input
/// is capped at 1 MiB.
pub fn parse(text: &str) -> Cfg {
    let text = truncate(text);
    let mut entries = Vec::new();
    let mut statement = String::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(character) = chars.next() {
        if quoted {
            statement.push(character);
            if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => {
                quoted = true;
                statement.push(character);
            }
            '/' if matches!(chars.peek(), Some('/')) => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            '/' if matches!(chars.peek(), Some('*')) => {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
            }
            '{' => {
                depth += 1;
                statement.push(character);
            }
            '}' => {
                depth -= 1;
                statement.push(character);
            }
            ';' if depth == 0 => {
                push(&mut entries, &statement);
                statement.clear();
            }
            _ => statement.push(character),
        }
    }
    push(&mut entries, &statement);
    Cfg { entries }
}

fn truncate(text: &str) -> &str {
    if text.len() <= MAX_BYTES {
        return text;
    }
    let mut end = MAX_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn push(entries: &mut Vec<(String, String)>, statement: &str) {
    let statement = statement.trim();
    let Some((key, value)) = statement.split_once('=') else {
        return;
    };
    let mut key = key.trim();
    if let Some(stripped) = key.strip_suffix("[]") {
        key = stripped.trim_end();
    }
    if key.is_empty() {
        return;
    }
    entries.push((key.to_string(), unquote(value.trim())));
}

fn unquote(value: &str) -> String {
    let unquoted = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'));
    unquoted.unwrap_or(value).to_string()
}

impl Cfg {
    /// The last value for a key, ignoring ASCII case.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }

    /// Every finding the checks produce. Passing and advisory values are
    /// included, because the caller prints one line per check.
    pub fn check(&self) -> Vec<CfgFinding> {
        let mut findings = Vec::new();
        findings.push(CfgFinding::VerifySignatures(
            self.get("verifySignatures").map(str::to_string),
        ));
        if let Some(value) = self.get("allowedFilePatching") {
            findings.push(CfgFinding::AllowedFilePatching(value.to_string()));
        }
        if let Some(value) = self.get("filePatchingExceptions") {
            if !array_entries(value).is_empty() {
                findings.push(CfgFinding::FilePatchingExceptions);
            }
        }
        if let Some(value) = self.get("equalModRequired") {
            findings.push(CfgFinding::EqualModRequired(value.to_string()));
        }
        findings
    }
}

/// The non-empty entries of a `{ "a", "b" }` value.
fn array_entries(value: &str) -> Vec<String> {
    let inner = value
        .trim()
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'));
    let Some(inner) = inner else {
        let value = unquote(value.trim());
        return if value.is_empty() {
            Vec::new()
        } else {
            vec![value]
        };
    };
    inner
        .split(',')
        .map(|entry| unquote(entry.trim()))
        .filter(|entry| !entry.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(text: &str) -> Vec<String> {
        parse(text)
            .check()
            .iter()
            .map(CfgFinding::describe)
            .collect()
    }

    #[test]
    fn the_baseline_passes() {
        let cfg = parse("battlEye = 1;\nverifySignatures = 2;\nallowedFilePatching = 0;\n");
        let findings = cfg.check();
        assert!(findings.iter().all(|finding| !finding.fatal()));
        assert_eq!(
            values("verifySignatures = 2;\nallowedFilePatching = 0;\n"),
            vec!["verifySignatures = 2", "allowedFilePatching = 0"]
        );
    }

    #[test]
    fn a_wrong_or_missing_verify_signatures_fails() {
        let wrong = parse("verifySignatures = 1;");
        assert!(wrong.check()[0].fatal());
        assert_eq!(wrong.check()[0].describe(), "verifySignatures = 1");

        let missing = parse("battlEye = 1;");
        assert!(missing.check()[0].fatal());
        assert_eq!(
            missing.check()[0].describe(),
            "verifySignatures is missing (the engine default is 0)"
        );
    }

    #[test]
    fn an_absent_allowed_file_patching_passes() {
        let cfg = parse("verifySignatures = 2;");
        assert!(cfg.check().iter().all(|finding| !finding.fatal()));
    }

    #[test]
    fn a_wrong_allowed_file_patching_fails() {
        let cfg = parse("verifySignatures = 2;\nallowedFilePatching = 2;");
        let findings = cfg.check();
        assert!(findings.iter().any(|finding| finding.fatal()));
        assert_eq!(findings[1].describe(), "allowedFilePatching = 2");
    }

    #[test]
    fn a_string_that_holds_a_setting_is_ignored() {
        let cfg = parse("hostname = \"verifySignatures = 0;\";\nverifySignatures = 2;");
        assert_eq!(cfg.get("verifySignatures"), Some("2"));
    }

    #[test]
    fn a_comment_that_holds_a_setting_is_ignored() {
        let cfg = parse("// verifySignatures = 0;\nverifySignatures = 2;");
        assert_eq!(cfg.get("verifySignatures"), Some("2"));

        let block = parse("/* verifySignatures = 0; */\nverifySignatures = 2;");
        assert_eq!(block.get("verifySignatures"), Some("2"));
    }

    #[test]
    fn file_patching_exceptions_warn_but_never_fail() {
        let cfg = parse(
            "verifySignatures = 2;\nallowedFilePatching = 0;\nfilePatchingExceptions[] = { \"7656119\" };",
        );
        let findings = cfg.check();
        assert!(findings.contains(&CfgFinding::FilePatchingExceptions));
        assert!(findings.iter().all(|finding| !finding.fatal()));
        assert_eq!(
            values("verifySignatures = 2;\nfilePatchingExceptions[] = { \"7656119\" };"),
            vec![
                "verifySignatures = 2",
                "filePatchingExceptions is not empty"
            ]
        );
    }

    #[test]
    fn an_empty_file_patching_list_does_not_warn() {
        let cfg = parse("filePatchingExceptions[] = {};");
        assert!(!cfg.check().contains(&CfgFinding::FilePatchingExceptions));
    }

    #[test]
    fn equal_mod_required_is_advisory() {
        let cfg = parse("verifySignatures = 2;\nequalModRequired = 0;");
        let findings = cfg.check();
        assert!(!findings.iter().any(CfgFinding::fatal));
        assert_eq!(
            findings.last().map(CfgFinding::describe).as_deref(),
            Some("equalModRequired = 0 (advisory)")
        );
    }

    #[test]
    fn only_depth_zero_statements_are_settings() {
        let cfg = parse("class Mission {\nverifySignatures = 0;\n};\nverifySignatures = 2;");
        assert_eq!(cfg.get("verifySignatures"), Some("2"));
    }

    #[test]
    fn keys_ignore_case_and_the_last_value_wins() {
        let cfg = parse("VERIFYSIGNATURES = 1;\nverifySignatures = 2;");
        assert_eq!(cfg.get("verifySignatures"), Some("2"));
    }
}
