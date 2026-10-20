//! Locate HEMTT projects.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::UksftaError;

const SKIP_DIRS: [&str; 14] = [
    ".git",
    "target",
    "node_modules",
    ".hemttout",
    "releases",
    ".uksf_tools",
    "addons",
    "keys",
    "include",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    "tests",
];

/// The current directory, or every project below `root`. A project is a
/// directory that holds `.hemtt/project.toml`.
pub fn find_projects(root: Option<&Path>) -> Result<Vec<PathBuf>, UksftaError> {
    match root {
        None => {
            let cwd = std::env::current_dir()?;
            if cwd.join(".hemtt").join("project.toml").is_file() {
                Ok(vec![cwd])
            } else {
                Ok(Vec::new())
            }
        }
        Some(root) => {
            if !root.is_dir() {
                return Err(UksftaError::Input(format!(
                    "{} is not a directory",
                    root.display()
                )));
            }
            let mut found = Vec::new();
            let walk = WalkDir::new(root).into_iter().filter_entry(|entry| {
                !entry.file_type().is_dir()
                    || !SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref())
            });
            for entry in walk {
                let entry = entry.map_err(|e| {
                    UksftaError::Input(format!("failed to scan {}: {e}", root.display()))
                })?;
                if entry.file_type().is_dir()
                    && entry.path().join(".hemtt").join("project.toml").is_file()
                {
                    found.push(entry.path().to_path_buf());
                }
            }
            found.sort();
            found.dedup();
            Ok(found)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_nested_projects_and_skips_build_dirs() {
        let root = std::env::temp_dir().join(format!("uksfta-docs-disc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = root.join("nested").join("pack");
        fs::create_dir_all(project.join(".hemtt")).unwrap();
        fs::write(project.join(".hemtt/project.toml"), "name = \"Pack\"\n").unwrap();

        let ignored = root.join("target").join("fake");
        fs::create_dir_all(ignored.join(".hemtt")).unwrap();
        fs::write(ignored.join(".hemtt/project.toml"), "name = \"Ignored\"\n").unwrap();

        let tools = root.join(".uksf_tools").join("template");
        fs::create_dir_all(tools.join(".hemtt")).unwrap();
        fs::write(tools.join(".hemtt/project.toml"), "name = \"Template\"\n").unwrap();

        let found = find_projects(Some(&root)).unwrap();
        assert_eq!(found, vec![project]);
    }

    #[test]
    fn missing_root_is_an_error() {
        assert!(find_projects(Some(Path::new("/nonexistent-uksfta-root"))).is_err());
    }
}
