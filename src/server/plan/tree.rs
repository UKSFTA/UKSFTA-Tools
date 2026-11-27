//! The bounded walk of the release root and the path checks that guard
//! every remote command.

use std::fs;
use std::path::{Path, PathBuf};

use super::join_remote;
use crate::error::UksftaError;

/// The most directory levels below the release root.
const MAX_DEPTH: usize = 8;
/// The most entries the walk visits.
const MAX_ENTRIES: usize = 10_000;
/// The most bytes in one path component.
const MAX_COMPONENT: usize = 255;

/// One file in the release root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEntry {
    pub rel: String,
    pub path: PathBuf,
    pub size: u64,
}

/// The whole release root.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    pub files: Vec<LocalEntry>,
    pub dirs: Vec<String>,
    pub skipped: usize,
}

/// Walk the release root, sorted by relative path.
///
/// # Errors
/// Returns `Input` when the root is missing or the depth or entry cap is
/// exceeded.
pub fn walk_release(dir: &Path) -> Result<Walk, UksftaError> {
    if !dir.is_dir() {
        return Err(UksftaError::Input(format!(
            "no release directory at {}",
            dir.display()
        )));
    }
    let mut walk = Walk::default();
    let mut visited = 0usize;
    walk_dir(dir, "", 0, &mut walk, &mut visited)?;
    walk.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    walk.dirs.sort();
    Ok(walk)
}

fn walk_dir(
    dir: &Path,
    prefix: &str,
    depth: usize,
    walk: &mut Walk,
    visited: &mut usize,
) -> Result<bool, UksftaError> {
    let items = fs::read_dir(dir).map_err(UksftaError::Io)?;
    let mut added = false;
    for item in items {
        let item = item.map_err(UksftaError::Io)?;
        *visited += 1;
        if *visited > MAX_ENTRIES {
            return Err(UksftaError::Input(format!(
                "the release tree has more than {MAX_ENTRIES} entries"
            )));
        }

        let name = item.file_name();
        let Some(name) = name.to_str() else {
            walk.skipped += 1;
            continue;
        };
        let rel = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        let kind = item.file_type().map_err(UksftaError::Io)?;

        if skip_name(name) || kind.is_symlink() {
            walk.skipped += 1;
            continue;
        }
        if kind.is_dir() {
            if depth + 1 > MAX_DEPTH {
                return Err(UksftaError::Input(format!(
                    "the release tree is deeper than {MAX_DEPTH} levels at {rel}"
                )));
            }
            if walk_dir(&item.path(), &rel, depth + 1, walk, visited)? {
                walk.dirs.push(rel);
                added = true;
            }
            continue;
        }
        if !kind.is_file() {
            walk.skipped += 1;
            continue;
        }
        let size = item.metadata().map_err(UksftaError::Io)?.len();
        walk.files.push(LocalEntry {
            rel,
            path: item.path(),
            size,
        });
        added = true;
    }
    Ok(added)
}

fn skip_name(name: &str) -> bool {
    if name.starts_with('.') {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".biprivatekey") || lower.ends_with(".tmp") || lower.ends_with(".bak")
}

/// Check a pack-relative path before it becomes a remote command.
///
/// # Errors
/// Returns `Input` for an empty, absolute, traversing or malformed path.
pub fn check_rel(rel: &str) -> Result<(), UksftaError> {
    let bad = rel.is_empty()
        || rel.starts_with('/')
        || rel.contains('\\')
        || rel.contains('"')
        || rel.chars().any(char::is_control)
        || rel.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.len() > MAX_COMPONENT
        });
    if bad {
        return Err(UksftaError::Input(format!(
            "refusing an unsafe relative path: {rel:?}"
        )));
    }
    Ok(())
}

/// Join a relative path inside the pack folder.
///
/// # Errors
/// Returns `Input` for an unsafe path or one that escapes the folder.
pub fn inside(pack_dir: &str, rel: &str) -> Result<String, UksftaError> {
    check_rel(rel)?;
    let joined = join_remote(pack_dir, rel);
    let prefix = format!("{}/", pack_dir.trim_end_matches('/'));
    if !joined.starts_with(&prefix) {
        return Err(UksftaError::Input(format!(
            "refusing a path outside the pack folder: {rel:?}"
        )));
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn write(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn the_walk_keeps_the_tree_and_counts_the_exclusions() {
        let root = tempfile::TempDir::new().unwrap();
        let base = root.path();
        write(&base.join("addons/a.pbo"), b"a");
        write(&base.join("keys/k.bikey"), b"k");
        write(&base.join("LICENSE"), b"l");
        write(&base.join("python_code/x.py"), b"x");
        write(&base.join(".DS_Store"), b"d");
        write(&base.join("sign.biprivatekey"), b"s");
        write(&base.join("x.tmp"), b"t");
        std::os::unix::fs::symlink("/etc/passwd", base.join("link")).unwrap();

        let walk = walk_release(base).unwrap();
        let rels: Vec<&str> = walk.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(
            rels,
            vec![
                "LICENSE",
                "addons/a.pbo",
                "keys/k.bikey",
                "python_code/x.py"
            ]
        );
        assert_eq!(walk.dirs, vec!["addons", "keys", "python_code"]);
        assert_eq!(walk.skipped, 4);
        assert!(rels.iter().all(|rel| !rel.starts_with('/')));
    }

    #[test]
    fn a_missing_root_is_an_input_error() {
        assert!(matches!(
            walk_release(Path::new("/nonexistent/release")),
            Err(UksftaError::Input(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_never_followed() {
        let root = tempfile::TempDir::new().unwrap();
        std::os::unix::fs::symlink("/etc", root.path().join("link")).unwrap();

        let walk = walk_release(root.path()).unwrap();
        assert!(walk.files.is_empty());
        assert!(walk.dirs.is_empty());
        assert_eq!(walk.skipped, 1);
    }

    #[test]
    fn check_rel_accepts_a_nested_file_and_refuses_the_rest() {
        assert!(check_rel("addons/a.pbo").is_ok());
        assert!(check_rel("python_code/x.py").is_ok());
        for value in [
            "", "/abs", "../x", "a//b", "a\\b", "a\"b", "./a", "a/./b", "a/../b",
        ] {
            assert!(check_rel(value).is_err(), "{value:?}");
        }
        let long = "a".repeat(300);
        assert!(check_rel(&long).is_err());
    }

    #[test]
    fn inside_joins_under_the_pack_and_refuses_traversal() {
        assert_eq!(inside("/srv/@1", "a/b").unwrap(), "/srv/@1/a/b");
        assert!(inside("/srv/@1", "../other/x").is_err());
    }
}
