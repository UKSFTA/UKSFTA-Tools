//! Parse a copy of the server launch line and find the pack entry.
//!
//! The panel exposes no readable startup file, so the line comes from a
//! local copy. The parser never executes anything and never expands a
//! variable.

/// The mod lists named on the launch line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModList {
    pub mods: Vec<String>,
    pub server_mods: Vec<String>,
    /// The `-par` value. An empty string means a bare `-par`.
    pub par_file: Option<String>,
}

/// The result of looking for one pack folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchCheck {
    /// The folder is on the line. `in_mod` is true for `-mod`.
    Satisfied { in_mod: bool },
    /// The folder is only present with different case.
    CaseMismatch { found: String },
    /// The folder is not on the line.
    Absent,
    /// The line hides the list, so the check cannot run.
    Unverifiable { reason: String },
    /// The folder is in both lists.
    Twice,
}

impl LaunchCheck {
    pub fn failed(&self) -> bool {
        matches!(
            self,
            LaunchCheck::Absent | LaunchCheck::CaseMismatch { .. } | LaunchCheck::Twice
        )
    }

    /// The report line, without the `launch: ` prefix.
    pub fn describe(&self, id: &str) -> String {
        match self {
            LaunchCheck::Satisfied { in_mod: true } => format!("@{id} is loaded"),
            LaunchCheck::Satisfied { in_mod: false } => format!("@{id} is not in -mod"),
            LaunchCheck::CaseMismatch { found } => format!("folder case differs: @{found}"),
            LaunchCheck::Absent => format!("@{id} is not loaded"),
            LaunchCheck::Unverifiable { reason } => reason.clone(),
            LaunchCheck::Twice => "loaded twice".to_string(),
        }
    }
}

/// Tokenise and read the two mod lists.
pub fn parse(line: &str) -> ModList {
    let mut list = ModList::default();
    for token in tokenize(line) {
        let lower = token.to_ascii_lowercase();
        if let Some(value) = token.get(5..).filter(|_| lower.starts_with("-mod=")) {
            list.mods.extend(split_value(value));
        } else if let Some(value) = token.get(11..).filter(|_| lower.starts_with("-servermod=")) {
            list.server_mods.extend(split_value(value));
        } else if lower == "-par" {
            list.par_file = Some(String::new());
        } else if let Some(value) = token.get(5..).filter(|_| lower.starts_with("-par=")) {
            list.par_file = Some(value.to_string());
        }
    }
    list
}

/// Find the pack folder on the parsed line.
pub fn check(list: &ModList, id: &str) -> LaunchCheck {
    if list.par_file.is_some() {
        return LaunchCheck::Unverifiable {
            reason: "-par hides the mod list".to_string(),
        };
    }
    let target = format!("@{id}");
    let in_mod = list
        .mods
        .iter()
        .any(|entry| last_component(entry) == target);
    let in_server = list
        .server_mods
        .iter()
        .any(|entry| last_component(entry) == target);

    if in_mod && in_server {
        return LaunchCheck::Twice;
    }
    if in_mod || in_server {
        return LaunchCheck::Satisfied { in_mod };
    }
    if let Some(found) = list
        .mods
        .iter()
        .chain(list.server_mods.iter())
        .find_map(|entry| case_only(entry, &target))
    {
        return LaunchCheck::CaseMismatch { found };
    }
    LaunchCheck::Absent
}

/// The last path component, with either separator.
fn last_component(value: &str) -> &str {
    value
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(value)
}

/// The folder name without `@` when it equals `target` ignoring case only.
fn case_only(entry: &str, target: &str) -> Option<String> {
    let name = last_component(entry);
    let rest = name.strip_prefix('@')?;
    (rest != target.trim_start_matches('@')
        && rest.eq_ignore_ascii_case(target.trim_start_matches('@')))
    .then(|| rest.to_string())
}

/// Split on `;` and unescape `\;` and `\"`.
fn split_value(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.next() {
                Some(next) if next == ';' || next == '"' => current.push(next),
                Some(next) => {
                    current.push('\\');
                    current.push(next);
                }
                None => current.push('\\'),
            }
        } else if character == ';' {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(character);
        }
    }
    out.push(current);
    out.retain(|entry| !entry.is_empty());
    out
}

/// Split on whitespace outside quotes. Quotes are removed. A backslash
/// escapes only `;` and `"`, so Windows paths stay intact.
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();

    while let Some(character) = chars.next() {
        match quote {
            Some(active) => {
                if character == '\\' {
                    if matches!(chars.peek(), Some(';' | '"')) {
                        current.push('\\');
                        current.push(chars.next().unwrap_or_default());
                    } else {
                        current.push('\\');
                    }
                } else if character == active {
                    quote = None;
                } else {
                    current.push(character);
                }
            }
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                } else if character.is_whitespace() {
                    if !current.is_empty() {
                        tokens.push(std::mem::take(&mut current));
                    }
                } else if character == '\\' {
                    if matches!(chars.peek(), Some(';' | '"')) {
                        current.push('\\');
                        current.push(chars.next().unwrap_or_default());
                    } else {
                        current.push('\\');
                    }
                } else {
                    current.push(character);
                }
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "3662499431";

    #[test]
    fn a_quoted_token_keeps_its_whitespace() {
        let list = parse("-mod=\"@a b;@c\" -serverMod=@d");
        assert_eq!(list.mods, vec!["@a b", "@c"]);
        assert_eq!(list.server_mods, vec!["@d"]);
    }

    #[test]
    fn an_unquoted_semicolon_splits_the_value() {
        let list = parse("-mod=@a;@b;@c");
        assert_eq!(list.mods, vec!["@a", "@b", "@c"]);
    }

    #[test]
    fn an_escaped_semicolon_stays_in_one_entry() {
        let list = parse("-mod=test\\;x\\test2;@b");
        assert_eq!(list.mods, vec!["test;x\\test2", "@b"]);
    }

    #[test]
    fn the_parameter_name_ignores_case() {
        let list = parse("-MOD=@a -ServerMod=@b");
        assert_eq!(list.mods, vec!["@a"]);
        assert_eq!(list.server_mods, vec!["@b"]);
    }

    #[test]
    fn a_par_file_hides_the_list() {
        assert_eq!(
            check(&parse("-par=server.par -mod=@1"), "1"),
            LaunchCheck::Unverifiable {
                reason: "-par hides the mod list".to_string()
            }
        );
        assert!(matches!(
            check(&parse("-par -mod=@1"), "1"),
            LaunchCheck::Unverifiable { .. }
        ));
    }

    #[test]
    fn the_exact_folder_satisfies_the_check() {
        assert_eq!(
            check(&parse("-mod=@1"), "1"),
            LaunchCheck::Satisfied { in_mod: true }
        );
        assert_eq!(
            check(&parse("-serverMod=mods/@1"), "1"),
            LaunchCheck::Satisfied { in_mod: false }
        );
    }

    #[test]
    fn a_prefix_does_not_satisfy_the_check() {
        assert_eq!(check(&parse("-mod=@10"), "1"), LaunchCheck::Absent);
        assert_eq!(check(&parse("-mod=@36624994310"), ID), LaunchCheck::Absent);
    }

    #[test]
    fn a_case_only_name_is_a_mismatch() {
        assert_eq!(
            check(&parse("-mod=@ONE"), "one"),
            LaunchCheck::CaseMismatch {
                found: "ONE".to_string()
            }
        );
    }

    #[test]
    fn the_folder_in_both_lists_is_twice() {
        assert_eq!(
            check(&parse("-mod=@1 -serverMod=@1"), "1"),
            LaunchCheck::Twice
        );
    }

    #[test]
    fn the_descriptions_are_the_report_lines() {
        assert_eq!(
            LaunchCheck::Satisfied { in_mod: true }.describe("1"),
            "@1 is loaded"
        );
        assert_eq!(
            LaunchCheck::Satisfied { in_mod: false }.describe("1"),
            "@1 is not in -mod"
        );
        assert_eq!(
            LaunchCheck::CaseMismatch {
                found: "One".to_string()
            }
            .describe("1"),
            "folder case differs: @One"
        );
        assert_eq!(LaunchCheck::Absent.describe("1"), "@1 is not loaded");
        assert_eq!(
            LaunchCheck::Unverifiable {
                reason: "-par hides the mod list".to_string()
            }
            .describe("1"),
            "-par hides the mod list"
        );
        assert_eq!(LaunchCheck::Twice.describe("1"), "loaded twice");
    }
}
