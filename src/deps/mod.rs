//! The `requiredAddons` closure of a built pack.
//!
//! Every `CfgPatches` class a PBO provides is matched against every
//! `requiredAddons` entry the pack declares, plus a mission's `addons[]`
//! list when one is given. The check is local and advisory: a class is
//! either provided by the pack, external (listed once by hand), or
//! unaccounted.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::UksftaError;
use crate::pbo::{config_list, pbo_closure_facts, pbo_entries, PBO_CPRS_MIME};

/// The hand-maintained list of addon classes the pack needs but does
/// not provide, usually base-game and DLC addons.
const EXTERNAL_LIST: &str = ".uksfta/external-addons.txt";

/// Largest `mission.sqm` entry the closure reads.
const MISSION_SQM_LIMIT: u64 = 16 * 1024 * 1024;

/// Where one requirement stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// A PBO in the pack provides the class.
    Provided,
    /// The external list names the class.
    External,
    /// Nothing in the pack or the list accounts for the class.
    Unaccounted,
}

/// One addon class that something requires. `from` names the requiring
/// file and keeps its own spelling, as does `addon`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub addon: String,
    pub from: String,
    pub state: State,
}

/// The whole closure: what the pack provides and what it requires.
#[derive(Debug, Clone)]
pub struct Closure {
    /// Lowercased class names the pack provides.
    pub provided: HashSet<String>,
    pub entries: Vec<Requirement>,
}

/// Options for one `uksfta deps` run.
pub struct DepsOptions<'a> {
    pub local: Option<&'a Path>,
    pub mission: Option<&'a Path>,
    pub external: Option<&'a Path>,
    pub strict: bool,
    pub suggest: bool,
}

/// Count the closure, print the summary, and fail under `--strict` when
/// any requirement is unaccounted.
pub fn run(options: &DepsOptions<'_>) -> Result<(), UksftaError> {
    let local = options.local.map_or_else(default_local, Path::to_path_buf);
    let external = match options.external {
        Some(path) => read_external(path)?,
        None => {
            let path = Path::new(EXTERNAL_LIST);
            if path.is_file() {
                read_external(path)?
            } else {
                HashSet::new()
            }
        }
    };

    let closure = analyse(&local, options.mission, &external)?;
    let unaccounted: Vec<&Requirement> = closure
        .entries
        .iter()
        .filter(|entry| entry.state == State::Unaccounted)
        .collect();

    println!("provided: {} addon class(es)", closure.provided.len());
    println!("requirements: {}", closure.entries.len());
    println!("unaccounted: {}", unaccounted.len());
    for entry in &unaccounted {
        println!("{} requires {}", entry.from, entry.addon);
    }

    if options.suggest {
        println!();
        println!("# {EXTERNAL_LIST}");
        println!("# Addon classes the pack needs but does not provide, one per line.");
        println!("# Lines that start with # are ignored.");
        for entry in &unaccounted {
            println!("{}", entry.addon);
        }
    }

    if options.strict && !unaccounted.is_empty() {
        return Err(UksftaError::Check(format!(
            "{} unaccounted addon requirement(s)",
            unaccounted.len()
        )));
    }
    Ok(())
}

/// Compute the closure for the pack under `local` and an optional
/// mission PBO. Matching is ASCII case-insensitive and the output keeps
/// the original spelling.
pub fn analyse(
    local: &Path,
    mission: Option<&Path>,
    external: &HashSet<String>,
) -> Result<Closure, UksftaError> {
    if !local.is_dir() {
        return Err(UksftaError::Input(format!(
            "no build directory at {}\nBuild the pack first, or pass --local PATH",
            local.display()
        )));
    }
    let addons = local.join("addons");
    if !addons.is_dir() {
        return Err(UksftaError::Input(format!(
            "no addons directory in {}",
            addons.display()
        )));
    }

    let external: HashSet<String> = external
        .iter()
        .map(|addon| addon.to_ascii_lowercase())
        .collect();
    let mut provided: HashSet<String> = HashSet::new();
    let mut required: Vec<(String, String)> = Vec::new();

    for path in pbo_paths(&addons) {
        let from = file_name(&path);
        let facts = pbo_closure_facts(&path);
        for class in facts.cfgpatches_classes {
            provided.insert(class.to_ascii_lowercase());
        }
        for addon in facts.required_addons {
            required.push((addon, from.clone()));
        }
    }

    if let Some(path) = mission {
        let text = read_mission_sqm(path)?;
        let from = file_name(path);
        for addon in config_list(&text, "addons[]") {
            required.push((addon, from.clone()));
        }
    }

    let mut entries: Vec<Requirement> = required
        .into_iter()
        .map(|(addon, from)| {
            let key = addon.to_ascii_lowercase();
            let state = if provided.contains(&key) {
                State::Provided
            } else if external.contains(&key) {
                State::External
            } else {
                State::Unaccounted
            };
            Requirement { addon, from, state }
        })
        .collect();
    entries.sort_by(|left, right| {
        left.from
            .to_ascii_lowercase()
            .cmp(&right.from.to_ascii_lowercase())
            .then_with(|| {
                left.addon
                    .to_ascii_lowercase()
                    .cmp(&right.addon.to_ascii_lowercase())
            })
    });

    Ok(Closure { provided, entries })
}

/// `.hemttout/release` when it exists, else `.hemttout/build`. The same
/// default as `uksfta server`.
fn default_local() -> PathBuf {
    let release = PathBuf::from(".hemttout/release");
    if release.is_dir() {
        release
    } else {
        PathBuf::from(".hemttout/build")
    }
}

/// Read the external addon list. Blank lines and `#` comments are
/// ignored, and the original spelling is kept.
fn read_external(path: &Path) -> Result<HashSet<String>, UksftaError> {
    let text = fs::read_to_string(path).map_err(|error| {
        UksftaError::Input(format!(
            "cannot read external addon list at {}: {error}",
            path.display()
        ))
    })?;
    let mut addons = HashSet::new();
    for line in text.lines() {
        let addon = line.split('#').next().unwrap_or("").trim();
        if !addon.is_empty() {
            addons.insert(addon.to_string());
        }
    }
    Ok(addons)
}

/// The `addons/*.pbo` files, sorted so the report is stable.
fn pbo_paths(addons: &Path) -> Vec<PathBuf> {
    let Ok(items) = fs::read_dir(addons) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = items
        .flatten()
        .map(|item| item.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pbo"))
        })
        .collect();
    paths.sort();
    paths
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Read a stored `mission.sqm` entry. A compressed entry carries a body
/// the tool cannot decode, and a missing entry is a wrong input, so both
/// are rejected with the reason.
fn read_mission_sqm(path: &Path) -> Result<String, UksftaError> {
    if !path.is_file() {
        return Err(UksftaError::Input(format!(
            "no mission PBO at {}",
            path.display()
        )));
    }
    let entry = pbo_entries(path)
        .into_iter()
        .find(|entry| {
            entry
                .name
                .rsplit(['\\', '/'])
                .next()
                .is_some_and(|stem| stem.eq_ignore_ascii_case("mission.sqm"))
        })
        .ok_or_else(|| {
            UksftaError::Input(format!(
                "no mission.sqm entry in {}\nPass a mission PBO, not a folder",
                path.display()
            ))
        })?;
    if entry.mime == PBO_CPRS_MIME {
        return Err(UksftaError::Input(format!(
            "mission.sqm in {} is compressed (Cprs)\nRepack the mission without compression",
            path.display()
        )));
    }
    if entry.mime != 0 {
        return Err(UksftaError::Input(format!(
            "mission.sqm in {} is not stored (mime {:#x})\nRepack the mission without compression or encryption",
            path.display(),
            entry.mime
        )));
    }
    if u64::from(entry.data_size) > MISSION_SQM_LIMIT {
        return Err(UksftaError::Input(format!(
            "mission.sqm in {} is larger than {} bytes",
            path.display(),
            MISSION_SQM_LIMIT
        )));
    }

    let mut file = fs::File::open(path)
        .map_err(|error| UksftaError::Input(format!("cannot read {}: {error}", path.display())))?;
    file.seek(SeekFrom::Start(entry.offset)).map_err(|error| {
        UksftaError::Input(format!("cannot seek in {}: {error}", path.display()))
    })?;
    let mut bytes = Vec::new();
    file.take(u64::from(entry.data_size))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            UksftaError::Input(format!(
                "cannot read mission.sqm from {}: {error}",
                path.display()
            ))
        })?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pbo::tests::write_pbo_entries;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(name);
        let addons = root.join("addons");
        fs::create_dir_all(&addons).unwrap();
        root
    }

    fn external_list(addons: &[&str]) -> HashSet<String> {
        addons.iter().map(|addon| addon.to_string()).collect()
    }

    #[test]
    fn closure_marks_provided_and_external_requirements() {
        let root = temp_root("uksfta-deps-provided");
        let addons = root.join("addons");
        write_pbo_entries(
            &addons.join("A.pbo"),
            &[(
                "config.cpp",
                0,
                br#"class CfgPatches { class a_main { requiredAddons[] = {"B_main", "A3_Data_F"}; }; };"#,
            )],
        );
        write_pbo_entries(
            &addons.join("B.pbo"),
            &[(
                "config.cpp",
                0,
                br#"class CfgPatches { class B_main {}; };"#,
            )],
        );

        let external = external_list(&["A3_Data_F"]);
        let closure = analyse(&root, None, &external).expect("analyse");

        assert_eq!(closure.provided.len(), 2);
        let states: Vec<(&str, State)> = closure
            .entries
            .iter()
            .map(|entry| (entry.addon.as_str(), entry.state))
            .collect();
        assert_eq!(
            states,
            [("A3_Data_F", State::External), ("B_main", State::Provided)]
        );
        assert!(closure.entries.iter().all(|entry| entry.from == "A.pbo"));

        write_pbo_entries(
            &addons.join("B.pbo"),
            &[("config.cpp", 0, br#"class CfgPatches {};"#)],
        );
        let closure = analyse(&root, None, &external).expect("analyse");
        assert!(closure
            .entries
            .iter()
            .any(|entry| entry.addon == "B_main" && entry.state == State::Unaccounted));

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn mission_addons_are_required() {
        let root = temp_root("uksfta-deps-mission");
        write_pbo_entries(
            &root.join("addons/B.pbo"),
            &[(
                "config.cpp",
                0,
                br#"class CfgPatches { class B_main {}; };"#,
            )],
        );
        let mission = root.join("mission.pbo");
        write_pbo_entries(
            &mission,
            &[(
                "mission.sqm",
                0,
                br#"class Mission { addons[] = {"B_main", "A3_Data_F", "Gone"}; };"#,
            )],
        );

        let external = external_list(&["A3_Data_F"]);
        let closure = analyse(&root, Some(&mission), &external).expect("analyse");

        for (addon, state) in [
            ("B_main", State::Provided),
            ("A3_Data_F", State::External),
            ("Gone", State::Unaccounted),
        ] {
            assert!(
                closure.entries.iter().any(|entry| entry.addon == addon
                    && entry.from == "mission.pbo"
                    && entry.state == state),
                "{addon} should be {state:?}"
            );
        }

        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn compressed_mission_is_an_input_error() {
        let root = temp_root("uksfta-deps-mission-compressed");
        let mission = root.join("mission.pbo");
        write_pbo_entries(
            &mission,
            &[("mission.sqm", PBO_CPRS_MIME, b"\x00compressed")],
        );

        let error = analyse(&root, Some(&mission), &HashSet::new()).unwrap_err();

        match error {
            UksftaError::Input(message) => assert!(message.contains("compressed")),
            other => panic!("expected an input error, got {other:?}"),
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_build_directory_is_an_input_error() {
        let local = std::env::temp_dir().join("uksfta-deps-absent-build");
        let error = analyse(&local, None, &HashSet::new()).unwrap_err();

        match error {
            UksftaError::Input(message) => {
                assert!(message.contains(&local.display().to_string()));
            }
            other => panic!("expected an input error, got {other:?}"),
        }
    }

    #[test]
    fn external_list_ignores_comments_and_blank_lines() {
        let dir = std::env::temp_dir().join("uksfta-deps-external");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("external.txt");
        fs::write(&path, "# base game\n\nA3_Data_F\n  ace_common  \n").unwrap();

        let addons = read_external(&path).expect("read the list");

        assert_eq!(addons.len(), 2);
        assert!(addons.contains("A3_Data_F"));
        assert!(addons.contains("ace_common"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
