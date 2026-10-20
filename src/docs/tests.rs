//! End-to-end tests over a throwaway HEMTT project. The changelog source is
//! `mods.lock`, so no git repository is needed.

use std::fs;
use std::path::PathBuf;

use super::{docs, DocsOptions};
use crate::error::UksftaError;

const PROJECT: &str = r#"
name = "Fixture Pack"
workshop_id = "9999999999"
workshop_tags = ["Mod"]

[version]
path = "addons/main/script_version.hpp"

[hemtt.publish]
description = "workshop_description.md"
changelog = "CHANGELOG.md"
"#;

const INITIAL_LOCK: &str = r#"{"version":1,"mods":{
  "111":{"files":["addons/alpha.pbo"],"name":"Alpha Mod","updated":"100"},
  "444":{"files":["addons/delta.pbo"],"name":"Delta Mod","updated":"100"},
  "222":{"files":[],"name":"Not Installed","updated":"100"}
}}"#;

const CHANGED_LOCK: &str = r#"{"version":1,"mods":{
  "111":{"files":["addons/alpha.pbo"],"name":"Alpha Mod","updated":"200"},
  "333":{"files":["addons/charlie.pbo"],"name":"Charlie Mod","updated":"300"}
}}"#;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("uksfta-docs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".hemtt")).unwrap();
        fs::create_dir_all(root.join("addons/main")).unwrap();
        fs::write(root.join(".hemtt/project.toml"), PROJECT).unwrap();
        fs::write(
            root.join("addons/main/script_version.hpp"),
            "#define MAJOR 1\n#define MINOR 0\n#define PATCHLVL 0\n#define BUILD 0\n",
        )
        .unwrap();
        fs::write(root.join("mods.lock"), INITIAL_LOCK).unwrap();
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn write_lock(&self, text: &str) {
        fs::write(self.path("mods.lock"), text).unwrap();
    }

    fn bump_version(&self) {
        fs::write(
            self.path("addons/main/script_version.hpp"),
            "#define MAJOR 2\n#define MINOR 0\n#define PATCHLVL 0\n#define BUILD 0\n",
        )
        .unwrap();
    }

    fn write(&self) -> DocsOptions<'_> {
        DocsOptions {
            root: Some(&self.root),
            dry_run: false,
            check: false,
            online: false,
            refresh_names: false,
            force: false,
        }
    }

    fn check(&self) -> DocsOptions<'_> {
        DocsOptions {
            root: Some(&self.root),
            dry_run: false,
            check: true,
            online: false,
            refresh_names: false,
            force: false,
        }
    }

    fn force(&self) -> DocsOptions<'_> {
        DocsOptions {
            root: Some(&self.root),
            dry_run: false,
            check: false,
            online: false,
            refresh_names: false,
            force: true,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn first_run_writes_the_description_and_an_initial_changelog() {
    let fixture = Fixture::new("initial");
    docs(&fixture.write()).unwrap();

    let description = fs::read_to_string(fixture.path("workshop_description.md")).unwrap();
    assert!(description.starts_with("# Fixture Pack\n\nVersion 1.0.0.0\n"));
    assert!(description.contains("- Alpha Mod (Workshop ID: 111)"));
    assert!(description.contains("- Delta Mod (Workshop ID: 444)"));
    assert!(!description.contains("Not Installed"));

    let changelog = fs::read_to_string(fixture.path("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("## [1.0.0] - "));
    assert!(changelog.contains("- Initial release with 2 mods."));
    assert!(fixture.path(".uksfta/publish-state.json").is_file());
}

#[test]
fn composition_changes_are_recorded_once_and_repeat_runs_are_stable() {
    let fixture = Fixture::new("changes");
    docs(&fixture.write()).unwrap();
    let initial = fs::read_to_string(fixture.path("CHANGELOG.md")).unwrap();

    fixture.write_lock(CHANGED_LOCK);
    docs(&fixture.write()).unwrap();

    let changelog = fs::read_to_string(fixture.path("CHANGELOG.md")).unwrap();
    assert!(changelog
        .contains("### Added\n\n- Initial release with 2 mods.\n- Charlie Mod (Workshop ID: 333)"));
    assert!(changelog.contains("### Changed\n\n- Alpha Mod (Workshop ID: 111)"));
    assert!(changelog.contains("### Removed\n\n- Delta Mod (Workshop ID: 444)"));
    assert!(changelog.contains("- Initial release with 2 mods."));
    assert_eq!(
        fs::read_to_string(fixture.path("CHANGELOG.md.bak")).unwrap(),
        initial
    );

    let before = (
        fs::read(fixture.path("CHANGELOG.md")).unwrap(),
        fs::read(fixture.path("workshop_description.md")).unwrap(),
        fs::read(fixture.path(".uksfta/publish-state.json")).unwrap(),
    );
    docs(&fixture.write()).unwrap();
    assert_eq!(
        before,
        (
            fs::read(fixture.path("CHANGELOG.md")).unwrap(),
            fs::read(fixture.path("workshop_description.md")).unwrap(),
            fs::read(fixture.path(".uksfta/publish-state.json")).unwrap(),
        )
    );

    let description = fs::read_to_string(fixture.path("workshop_description.md")).unwrap();
    assert!(description.contains("- Charlie Mod (Workshop ID: 333)"));
    assert!(!description.contains("Delta Mod"));

    docs(&fixture.check()).unwrap();
}

#[test]
fn a_version_bump_starts_a_new_section_and_keeps_history() {
    let fixture = Fixture::new("bump");
    docs(&fixture.write()).unwrap();
    fixture.write_lock(CHANGED_LOCK);
    docs(&fixture.write()).unwrap();

    fixture.bump_version();
    docs(&fixture.write()).unwrap();

    let changelog = fs::read_to_string(fixture.path("CHANGELOG.md")).unwrap();
    let new_at = changelog.find("## [2.0.0]").unwrap();
    let old_at = changelog.find("## [1.0.0]").unwrap();
    assert!(new_at < old_at);
    assert!(changelog.contains("_No changes recorded._"));
    assert!(changelog.contains("- Charlie Mod (Workshop ID: 333)"));
}

#[test]
fn check_fails_when_the_composition_changed_after_the_last_run() {
    let fixture = Fixture::new("check");
    docs(&fixture.write()).unwrap();
    docs(&fixture.check()).unwrap();

    fixture.write_lock(CHANGED_LOCK);
    let err = docs(&fixture.check()).unwrap_err();
    assert!(matches!(err, UksftaError::Check(_)));
    assert_eq!(err.exit_code(), 1);
    assert!(err.to_string().contains("CHANGELOG.md"));

    fs::remove_file(fixture.path("workshop_description.md")).unwrap();
    assert!(matches!(
        docs(&fixture.check()).unwrap_err(),
        UksftaError::Check(_)
    ));
}

#[test]
fn dry_run_writes_nothing_and_leaves_no_state() {
    let fixture = Fixture::new("dry");
    let options = DocsOptions {
        root: Some(&fixture.root),
        dry_run: true,
        check: false,
        online: false,
        refresh_names: false,
        force: false,
    };
    docs(&options).unwrap();

    assert!(!fixture.path("workshop_description.md").exists());
    assert!(!fixture.path("CHANGELOG.md").exists());
    assert!(!fixture.path(".uksfta/publish-state.json").exists());
}

#[test]
fn honours_publish_paths_with_any_name_or_depth() {
    let fixture = Fixture::new("paths");
    fs::write(
        fixture.path(".hemtt/project.toml"),
        "name = \"Fixture Pack\"\n[version]\npath = \"addons/main/script_version.hpp\"\n[hemtt.publish]\ndescription = \".hemtt/workshop.md\"\nchangelog = \"docs/changes.md\"\n",
    )
    .unwrap();

    docs(&fixture.write()).unwrap();

    assert!(fixture.path(".hemtt/workshop.md").is_file());
    assert!(fixture.path("docs/changes.md").is_file());
    assert!(!fixture.path("workshop_description.md").exists());
    assert!(!fixture.path("CHANGELOG.md").exists());
    docs(&fixture.check()).unwrap();
}

#[test]
fn project_without_publish_changes_nothing() {
    let fixture = Fixture::new("nopublish");
    fs::write(
        fixture.path(".hemtt/project.toml"),
        "name = \"Fixture Pack\"\n[version]\npath = \"addons/main/script_version.hpp\"\n",
    )
    .unwrap();

    docs(&fixture.write()).unwrap();
    assert!(!fixture.path("workshop_description.md").exists());
    assert!(!fixture.path("CHANGELOG.md").exists());
}

#[test]
fn a_missing_lock_skips_the_project() {
    let fixture = Fixture::new("nolock");
    fs::remove_file(fixture.path("mods.lock")).unwrap();

    docs(&fixture.write()).unwrap();
    assert!(!fixture.path("workshop_description.md").exists());
    assert!(!fixture.path("CHANGELOG.md").exists());
}

#[test]
fn generated_files_carry_the_marker() {
    let fixture = Fixture::new("marker");
    docs(&fixture.write()).unwrap();
    assert!(fs::read_to_string(fixture.path("workshop_description.md"))
        .unwrap()
        .contains(super::MARKER));
    assert!(fs::read_to_string(fixture.path("CHANGELOG.md"))
        .unwrap()
        .contains(super::MARKER));
}

#[test]
fn hand_written_files_are_protected_without_force() {
    let fixture = Fixture::new("handwritten");
    fs::write(fixture.path("workshop_description.md"), "# Hand written\n").unwrap();
    fs::write(
        fixture.path("CHANGELOG.md"),
        "# Changelog\n\n## [1.0.0] - 2026-01-01\n\n### Added\n\n- Hand written\n",
    )
    .unwrap();

    docs(&fixture.write()).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.path("workshop_description.md")).unwrap(),
        "# Hand written\n"
    );
    assert!(fs::read_to_string(fixture.path("CHANGELOG.md"))
        .unwrap()
        .contains("- Hand written"));
    assert!(matches!(
        docs(&fixture.check()).unwrap_err(),
        UksftaError::Check(_)
    ));

    docs(&fixture.force()).unwrap();
    assert!(fs::read_to_string(fixture.path("workshop_description.md"))
        .unwrap()
        .starts_with("# Fixture Pack"));
    let changelog = fs::read_to_string(fixture.path("CHANGELOG.md")).unwrap();
    assert!(changelog.contains("- Initial release with 2 mods."));
    assert!(!changelog.contains("- Hand written"));
}

#[test]
fn missing_project_file_is_an_input_error() {
    let root = std::env::temp_dir().join(format!("uksfta-docs-empty-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let options = DocsOptions {
        root: Some(root.as_path()),
        dry_run: false,
        check: false,
        online: false,
        refresh_names: false,
        force: false,
    };
    let err = docs(&options).unwrap_err();
    assert!(matches!(err, UksftaError::Input(_)));
    assert_eq!(err.exit_code(), 2);
    let _ = fs::remove_dir_all(&root);
}
