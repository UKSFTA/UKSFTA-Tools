//! Parse the parts of `.hemtt/project.toml` this command reads: the
//! project name, the version source, and the `[hemtt.publish]` targets.
//! The shape is HEMTT 1.22.0. Unknown keys, including the legacy
//! top-level `workshop_id` and `workshop_tags`, are ignored.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::error::UksftaError;

#[derive(Debug, Deserialize)]
struct RawProject {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<RawVersion>,
    #[serde(default)]
    hemtt: Option<RawHemtt>,
}

#[derive(Debug, Deserialize)]
struct RawVersion {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    git_hash: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct RawHemtt {
    #[serde(default)]
    publish: Option<RawPublish>,
}

#[derive(Debug, Deserialize)]
struct RawPublish {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    changelog: Option<String>,
}

/// The subset of `.hemtt/project.toml` this tool needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectConfig {
    pub name: Option<String>,
    pub version_path: Option<PathBuf>,
    pub git_hash: Option<u8>,
    pub publish: Option<PublishConfig>,
}

/// The publish targets named by `[hemtt.publish]`. Both keys are optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PublishConfig {
    pub description: Option<String>,
    pub changelog: Option<String>,
}

impl PublishConfig {
    /// True when the section names no file, so there is nothing to render.
    pub fn is_empty(&self) -> bool {
        self.description.is_none() && self.changelog.is_none()
    }
}

/// A present but empty key counts as absent, so it names no file.
fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

/// Parse a project file. A syntax error is a hard error: a project whose
/// file cannot be read is not silently skipped.
pub fn parse_project(text: &str) -> Result<ProjectConfig, UksftaError> {
    let raw: RawProject = toml::from_str(text)
        .map_err(|e| UksftaError::Parse(format!("invalid HEMTT project file: {e}")))?;
    let git_hash = raw.version.as_ref().and_then(|version| version.git_hash);
    Ok(ProjectConfig {
        name: non_empty(raw.name),
        version_path: non_empty(raw.version.and_then(|v| v.path)).map(PathBuf::from),
        git_hash,
        publish: raw
            .hemtt
            .and_then(|h| h.publish)
            .map(|publish| PublishConfig {
                description: non_empty(publish.description),
                changelog: non_empty(publish.changelog),
            }),
    })
}

/// Read `.hemtt/project.toml` below `root`.
pub fn load_project(root: &Path) -> Result<ProjectConfig, UksftaError> {
    let path = root.join(".hemtt").join("project.toml");
    let text = fs::read_to_string(&path)
        .map_err(|e| UksftaError::Input(format!("cannot read {}: {e}", path.display())))?;
    parse_project(&text)
}

/// Resolve a publish file path against the project root. An absolute path
/// or a parent traversal is rejected, so a config cannot write outside the
/// project.
pub fn resolve_path(root: &Path, relative: &str) -> Result<PathBuf, UksftaError> {
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(UksftaError::Parse(format!(
            "publish path must be relative: {relative}"
        )));
    }
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(UksftaError::Parse(format!(
            "publish path must stay inside the project: {relative}"
        )));
    }
    Ok(root.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: &str = r#"
name = "Project"
workshop_id = "3662499431"
workshop_tags = ["Mod"]

[version]
path = "addons/main/script_version.hpp"

[hemtt.publish]
description = "workshop_description.md"
changelog = "CHANGELOG.md"
"#;

    #[test]
    fn parses_both_publish_keys() {
        let config = parse_project(BOTH).unwrap();
        assert_eq!(config.name.as_deref(), Some("Project"));
        assert_eq!(
            config.version_path.as_deref(),
            Some(Path::new("addons/main/script_version.hpp"))
        );
        assert_eq!(
            config.publish,
            Some(PublishConfig {
                description: Some("workshop_description.md".to_string()),
                changelog: Some("CHANGELOG.md".to_string()),
            })
        );
    }

    #[test]
    fn parses_one_publish_key() {
        let config = parse_project("[hemtt.publish]\ndescription = \"d.md\"\n").unwrap();
        let publish = config.publish.unwrap();
        assert_eq!(publish.description.as_deref(), Some("d.md"));
        assert_eq!(publish.changelog, None);
        assert!(!publish.is_empty());
    }

    #[test]
    fn missing_publish_section_is_none() {
        let config = parse_project("name = \"Pack\"\n[version]\npath = \"v.hpp\"\n").unwrap();
        assert_eq!(config.publish, None);
        assert_eq!(config.version_path.as_deref(), Some(Path::new("v.hpp")));
    }

    #[test]
    fn empty_publish_section_has_no_targets() {
        let config = parse_project("[hemtt.publish]\ndescription = \"\"\n").unwrap();
        assert!(config.publish.unwrap().is_empty());
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(matches!(
            parse_project("name = [unterminated"),
            Err(UksftaError::Parse(_))
        ));
    }

    #[test]
    fn legacy_workshop_keys_are_ignored() {
        let config =
            parse_project("name = \"X\"\nworkshop_id = \"1\"\nworkshop_tags = []\n").unwrap();
        assert_eq!(config.name.as_deref(), Some("X"));
        assert_eq!(config.publish, None);
    }

    #[test]
    fn rejects_absolute_and_parent_paths() {
        let root = Path::new("/project");
        assert!(resolve_path(root, "../outside.md").is_err());
        assert!(resolve_path(root, "/etc/passwd").is_err());
        assert_eq!(
            resolve_path(root, "docs/file.md").unwrap(),
            PathBuf::from("/project/docs/file.md")
        );
    }
}
