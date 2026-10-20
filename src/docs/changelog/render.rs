//! Render and merge Keep a Changelog sections.

use super::diff::{Changes, ModChange};

const HEADER: &str = "# Changelog\n\nAll notable changes to this project are documented in this file.\n\nThe format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).\n";

/// tool. A hand-written section is left alone unless the caller forces it.
pub fn can_replace(existing: Option<&str>, version: &str) -> bool {
    let Some(text) = existing else {
        return true;
    };
    let (_, sections) = split_sections(text);
    match sections
        .iter()
        .find(|section| section_version(section) == Some(version))
    {
        Some(section) => section.contains(super::super::MARKER),
        None => true,
    }
}

/// Merge a generated section into an existing changelog.
pub fn render_changelog(
    existing: Option<&str>,
    version: &str,
    date: &str,
    changes: &Changes,
) -> String {
    let (intro, mut sections) = match existing {
        Some(text) if !text.trim().is_empty() => split_sections(text),
        _ => (HEADER.trim_end().to_string(), Vec::new()),
    };

    let section_date = sections
        .iter()
        .find(|text| section_version(text) == Some(version))
        .and_then(|text| section_date(text))
        .unwrap_or(date);
    let section = build_section(version, section_date, changes);

    match sections
        .iter()
        .position(|text| section_version(text) == Some(version))
    {
        Some(index) => sections[index] = section,
        None => sections.insert(0, section),
    }
    render_parts(&intro, &sections)
}

/// Split a changelog into its intro and its version sections, newest first.
fn split_sections(text: &str) -> (String, Vec<String>) {
    let mut intro = String::new();
    let mut sections: Vec<String> = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("## ") {
            sections.push(String::new());
        }
        match sections.last_mut() {
            Some(section) => {
                section.push_str(line);
                section.push('\n');
            }
            None => {
                intro.push_str(line);
                intro.push('\n');
            }
        }
    }
    let sections = sections
        .iter()
        .map(|section| section.trim_end().to_string())
        .collect();
    (intro.trim_end().to_string(), sections)
}

/// Join the intro and the sections with one blank line between blocks.
fn render_parts(intro: &str, sections: &[String]) -> String {
    let mut out = String::from(intro);
    for section in sections {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(section);
    }
    out.push('\n');
    out
}

/// The version named by a section's `## [x] - date` heading.
fn section_version(section: &str) -> Option<&str> {
    let first = section.lines().next()?;
    let rest = first.trim_start().strip_prefix("## [")?;
    let end = rest.find(']')?;
    Some(&rest[..end])
}

/// The date of an existing heading, so a repeated run keeps it.
fn section_date(section: &str) -> Option<&str> {
    let first = section.lines().next()?;
    let after = first.split_once("] - ")?.1.trim();
    if after.is_empty() {
        None
    } else {
        Some(after)
    }
}

/// Render one version section, with no trailing blank line.
fn build_section(version: &str, date: &str, changes: &Changes) -> String {
    let mut out = format!("## [{version}] - {date}\n");

    let mut added: Vec<String> = Vec::new();
    if let Some(count) = changes.initial {
        let noun = if count == 1 { "mod" } else { "mods" };
        added.push(format!("Initial release with {count} {noun}."));
    }
    added.extend(changes.set.added.iter().map(item_line));
    let changed: Vec<String> = changes.set.updated.iter().map(updated_line).collect();
    let removed: Vec<String> = changes.set.removed.iter().map(item_line).collect();

    let mut wrote_any = false;
    for (heading, items) in [("Added", added), ("Changed", changed), ("Removed", removed)] {
        if items.is_empty() {
            continue;
        }
        out.push_str(&format!("\n### {heading}\n\n"));
        for item in items {
            out.push_str(&format!("- {item}\n"));
        }
        wrote_any = true;
    }
    if !wrote_any {
        out.push_str("\n_No changes recorded._\n");
    }
    out.push_str(&format!("\n{}\n", super::super::MARKER));
    out.trim_end().to_string()
}

fn item_line(item: &ModChange) -> String {
    format!("{} (Workshop ID: {})", item.name, item.id)
}

/// A changed item includes the Workshop update date when the lock has one.
fn updated_line(item: &ModChange) -> String {
    let line = item_line(item);
    match item
        .updated
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok())
    {
        Some(seconds) => format!(
            "{line}, updated {}",
            super::super::date::date_from_unix_secs(seconds)
        ),
        None => line,
    }
}
