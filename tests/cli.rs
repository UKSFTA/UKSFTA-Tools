//! End-to-end tests for the command-line surface. Each test runs the real
//! binary in a temporary directory, so no test touches the working tree and
//! no test uses the network. `UKSFTA_NO_UPDATE_CHECK` keeps the automatic
//! update check out of every run.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// One command in `dir` with the update check disabled.
fn uksfta(dir: &Path) -> Command {
    let mut command = Command::cargo_bin("uksfta").expect("the binary must build");
    command.current_dir(dir).env("UKSFTA_NO_UPDATE_CHECK", "1");
    command
}

fn write(path: &Path, content: &[u8]) {
    fs::write(path, content).expect("write the fixture file");
}

/// A TOML mod_sources.txt with one entry.
const SOURCES: &str = "version = 2\n\n[[mods]]\nid = \"450814997\"\nname = \"CBA_A3\"\n";

/// A launcher preset with one Steam mod.
const MODLIST: &str = r#"<html><body><table>
<tr data-type="ModContainer">
<td data-type="DisplayName">O&amp;T Warfighters</td>
<td><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=1234567890" data-type="Link">URL</a></td>
</tr>
</table></body></html>"#;

#[test]
fn help_exits_zero_and_lists_the_commands() {
    let dir = TempDir::new().expect("temp dir");

    uksfta(dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("export"))
        .stdout(predicate::str::contains("size"));
}

#[test]
fn verify_without_a_lock_exits_two_and_names_the_lock_file() {
    let dir = TempDir::new().expect("temp dir");

    uksfta(dir.path())
        .arg("verify")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("mods.lock"));
}

#[test]
fn size_sorts_by_name_and_reports_the_tracked_total() {
    let dir = TempDir::new().expect("temp dir");
    fs::create_dir(dir.path().join("addons")).expect("create addons");
    write(&dir.path().join("addons/Alpha.pbo"), &[0u8; 1024]);
    write(&dir.path().join("addons/Zulu.pbo"), &[0u8; 2048]);
    write(
        &dir.path().join("mods.lock"),
        br#"{"version":1,"mods":{
            "2222222222":{"name":"Alpha Mod","files":["addons/Alpha.pbo"]},
            "1111111111":{"name":"Zulu Mod","files":["addons/Zulu.pbo"]}
        }}"#,
    );

    let assert = uksfta(dir.path())
        .args(["size", "--sort", "name"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 output");

    let alpha = stdout.find("Alpha Mod").expect("Alpha Mod is listed");
    let zulu = stdout.find("Zulu Mod").expect("Zulu Mod is listed");
    assert!(alpha < zulu, "name order must be Alpha before Zulu");
    assert!(stdout.contains("2 mod(s)"), "both mods are counted");
    assert!(stdout.contains("3.1 KB"), "the tracked total is 3072 bytes");
}

#[test]
fn import_appends_a_mod_and_keeps_the_original_as_bak() {
    let dir = TempDir::new().expect("temp dir");
    write(&dir.path().join("mod_sources.txt"), SOURCES.as_bytes());
    write(&dir.path().join("modlist.html"), MODLIST.as_bytes());

    uksfta(dir.path())
        .args(["import", "modlist.html"])
        .assert()
        .success();

    let sources = fs::read_to_string(dir.path().join("mod_sources.txt")).expect("read sources");
    assert!(sources.contains("1234567890"), "the imported id is written");
    let backup = fs::read_to_string(dir.path().join("mod_sources.txt.bak")).expect("read backup");
    assert_eq!(
        backup, SOURCES,
        "the backup holds the original content only"
    );
}

#[test]
fn export_dry_run_prints_the_mod_and_writes_no_file() {
    let dir = TempDir::new().expect("temp dir");
    write(&dir.path().join("mod_sources.txt"), SOURCES.as_bytes());

    uksfta(dir.path())
        .args(["export", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("450814997"));

    assert!(
        !dir.path().join("modlist.html").exists(),
        "a dry run must not write the preset"
    );
}

#[test]
fn size_pbo_lists_an_untracked_pbo_with_its_size() {
    let dir = TempDir::new().expect("temp dir");
    fs::create_dir(dir.path().join("addons")).expect("create addons");
    write(&dir.path().join("addons/extra.pbo"), &[0u8; 2048]);

    uksfta(dir.path())
        .args(["size", "--pbo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("extra.pbo"))
        .stdout(predicate::str::contains("2.0 KB"));
}
