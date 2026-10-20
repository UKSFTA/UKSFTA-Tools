//! Resolve the project version. The HEMTT `[version] path` script header
//! wins; a `VERSION` file in the project root is the fallback.

use std::fmt;
use std::fs;
use std::path::Path;

/// A HEMTT script version. The build number is optional, because HEMTT
/// omits it from the display form when `script_version.hpp` has no `BUILD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub build: Option<u32>,
}

impl fmt::Display for ProjectVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(build) = self.build {
            write!(f, ".{build}")?;
        }
        Ok(())
    }
}

impl ProjectVersion {
    /// The three-component form HEMTT's changelog parser accepts.
    #[must_use]
    pub fn short(&self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Read one `#define KEY value` from a script header. A trailing `//`
/// comment is ignored. Returns None when the key is absent or not a number.
fn define_value(text: &str, key: &str) -> Option<u32> {
    text.lines().find_map(|line| {
        let code = line.split("//").next().unwrap_or_default().trim();
        let mut parts = code.split_whitespace();
        if parts.next()? != "#define" {
            return None;
        }
        if parts.next()? != key {
            return None;
        }
        parts.next()?.parse().ok()
    })
}

/// Parse `#define MAJOR`, `MINOR`, `PATCHLVL`, and `BUILD` from a script
/// version header. MAJOR and MINOR are required; a missing PATCHLVL or
/// BUILD counts as zero.
pub fn parse_script_version(text: &str) -> Option<ProjectVersion> {
    Some(ProjectVersion {
        major: define_value(text, "MAJOR")?,
        minor: define_value(text, "MINOR")?,
        patch: define_value(text, "PATCHLVL")
            .or_else(|| define_value(text, "PATCH"))
            .unwrap_or(0),
        build: define_value(text, "BUILD"),
    })
}

/// Parse a dotted version file such as `1.2.3` or `1.2.3.4`. Missing
/// components count as zero. More than four components is not a version.
pub fn parse_version_file(text: &str) -> Option<ProjectVersion> {
    let trimmed = text.trim();
    let trimmed = trimmed
        .strip_prefix('v')
        .or_else(|| trimmed.strip_prefix('V'))
        .unwrap_or(trimmed);
    let mut numbers = [0u32; 4];
    let mut components = 0;
    for (index, part) in trimmed.split('.').enumerate() {
        if index == numbers.len() {
            return None;
        }
        numbers[index] = part.trim().parse().ok()?;
        components = index + 1;
    }
    Some(ProjectVersion {
        major: numbers[0],
        minor: numbers[1],
        patch: numbers[2],
        build: (components == 4).then_some(numbers[3]),
    })
}

/// Resolve the project version from the configured header, then from a
/// `VERSION` file. None means the project declares no usable version.
pub fn resolve_version(root: &Path, configured: Option<&Path>) -> Option<ProjectVersion> {
    if let Some(path) = configured {
        if let Ok(text) = fs::read_to_string(root.join(path)) {
            if let Some(version) = parse_script_version(&text) {
                return Some(version);
            }
        }
    }
    let text = fs::read_to_string(root.join("VERSION")).ok()?;
    parse_version_file(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HPP: &str = "#define MAJOR 1\n#define MINOR 2\n#define PATCHLVL 3\n#define BUILD 4\n";

    #[test]
    fn parses_all_four_defines() {
        assert_eq!(
            parse_script_version(HPP).map(|v| v.to_string()),
            Some("1.2.3.4".to_string())
        );
    }

    #[test]
    fn parses_defines_with_comments_and_spacing() {
        let text = "\t#define  MAJOR 7 // major\n#define MINOR 8//minor\n#define PATCHLVL 9\n#define BUILD 10\n";
        assert_eq!(
            parse_script_version(text).map(|v| v.to_string()),
            Some("7.8.9.10".to_string())
        );
    }

    #[test]
    fn missing_build_is_omitted() {
        assert_eq!(
            parse_script_version("#define MAJOR 2\n#define MINOR 7\n#define PATCHLVL 1\n")
                .map(|v| v.to_string()),
            Some("2.7.1".to_string())
        );
    }

    #[test]
    fn short_version_drops_the_build() {
        assert_eq!(
            parse_script_version(HPP).map(|v| v.short()),
            Some("1.2.3".to_string())
        );
    }

    #[test]
    fn rejects_header_without_major() {
        assert_eq!(parse_script_version("#define MINOR 7\n"), None);
    }

    #[test]
    fn rejects_header_with_non_numeric_value() {
        assert_eq!(
            parse_script_version("#define MAJOR one\n#define MINOR 2\n"),
            None
        );
    }

    #[test]
    fn parses_version_file_with_three_components() {
        assert_eq!(
            parse_version_file("3.4.5\n").map(|v| v.to_string()),
            Some("3.4.5".to_string())
        );
    }

    #[test]
    fn parses_version_file_with_four_components_and_prefix() {
        assert_eq!(
            parse_version_file(" v1.2.3.4 ").map(|v| v.to_string()),
            Some("1.2.3.4".to_string())
        );
    }

    #[test]
    fn rejects_version_file_with_five_components() {
        assert_eq!(parse_version_file("1.2.3.4.5"), None);
    }

    #[test]
    fn rejects_empty_version_file() {
        assert_eq!(parse_version_file("\n"), None);
    }

    #[test]
    fn hpp_file_wins_over_version_file() {
        let root = std::env::temp_dir().join(format!("uksfta-docs-ver-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("addons/main")).unwrap();
        fs::write(root.join("addons/main/script_version.hpp"), HPP).unwrap();
        fs::write(root.join("VERSION"), "9.9.9\n").unwrap();

        let version = resolve_version(&root, Some(Path::new("addons/main/script_version.hpp")));
        assert_eq!(version.map(|v| v.to_string()), Some("1.2.3.4".to_string()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn version_file_is_the_fallback() {
        let root = std::env::temp_dir().join(format!("uksfta-docs-ver2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("VERSION"), "3.4.5\n").unwrap();

        let version = resolve_version(&root, Some(Path::new("missing.hpp")));
        assert_eq!(version.map(|v| v.to_string()), Some("3.4.5".to_string()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn no_version_source_is_none() {
        let root = std::env::temp_dir().join(format!("uksfta-docs-ver3-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        assert_eq!(resolve_version(&root, None), None);
        fs::remove_dir_all(&root).unwrap();
    }
}
