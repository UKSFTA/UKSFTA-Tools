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

#[cfg(unix)]
fn notify_stub(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let stub = dir.join("notify.sh");
    write(
        &stub,
        b"#!/bin/sh\nprintf '%s\\n' \"$1\" > \"$UKSFTA_NOTIFY_OUT\"\nprintf '%s\\n' \"$2\" >> \"$UKSFTA_NOTIFY_OUT\"\n",
    );
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).expect("make the stub runnable");
    stub
}

#[cfg(unix)]
#[test]
fn a_failed_run_notifies_only_with_the_flag() {
    let dir = TempDir::new().expect("temp dir");
    let stub = notify_stub(dir.path());
    let out = dir.path().join("notify.out");

    uksfta(dir.path())
        .arg("server")
        .env("UKSFTA_NOTIFY_BIN", &stub)
        .env("UKSFTA_NOTIFY_OUT", &out)
        .assert()
        .code(2);
    assert!(!out.exists(), "no notification without --notify");

    uksfta(dir.path())
        .args(["server", "--notify"])
        .env("UKSFTA_NOTIFY_BIN", &stub)
        .env("UKSFTA_NOTIFY_OUT", &out)
        .assert()
        .code(2);
    let text = fs::read_to_string(&out).expect("the notification body");
    assert!(text.starts_with("uksfta\n"), "{text}");
    assert!(text.contains("failed:"), "{text}");
}

#[test]
fn server_help_lists_the_limit_and_notification_flags() {
    let dir = TempDir::new().expect("temp dir");

    uksfta(dir.path())
        .args(["server", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--limit"))
        .stdout(predicate::str::contains("--preallocate"))
        .stdout(predicate::str::contains("--no-preallocate"))
        .stdout(predicate::str::contains("--notify"));
}

#[test]
fn help_exits_zero_and_lists_the_commands() {
    let dir = TempDir::new().expect("temp dir");

    uksfta(dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("export"))
        .stdout(predicate::str::contains("size"))
        .stdout(predicate::str::contains("deps"));
}

#[test]
fn server_help_lists_the_opt_in_config_checks() {
    let dir = TempDir::new().expect("temp dir");

    uksfta(dir.path())
        .args(["server", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--launch-line"))
        .stdout(predicate::str::contains("--server-cfg"))
        .stdout(predicate::str::contains("--strict-config"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--doctor"));
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

/// A PBO with stored text entries, enough for the addon closure scan.
fn write_pbo(path: &Path, entries: &[(&str, &str)]) {
    let properties = b"prefix\0test\0\0";
    let mut bytes = Vec::new();
    bytes.push(0);
    bytes.extend(0x5665_7273u32.to_le_bytes()); // Vers
    for _ in 0..3 {
        bytes.extend(0u32.to_le_bytes());
    }
    bytes.extend((properties.len() as u32).to_le_bytes());
    bytes.extend(properties);
    for (name, content) in entries {
        bytes.extend(name.as_bytes());
        bytes.push(0);
        bytes.extend(0u32.to_le_bytes()); // stored
        let size = content.len() as u32;
        bytes.extend(size.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(0u32.to_le_bytes());
        bytes.extend(size.to_le_bytes());
    }
    bytes.extend([0u8; 21]);
    for (_, content) in entries {
        bytes.extend(content.as_bytes());
    }
    write(path, &bytes);
}

/// A build tree with A (requires B_main and A3_Data_F) and B (provides
/// B_main). The optional text becomes `.uksfta/external-addons.txt`.
fn deps_fixture(dir: &Path, external: Option<&str>) {
    let addons = dir.join(".hemttout/release/addons");
    fs::create_dir_all(&addons).expect("create addons");
    write_pbo(
        &addons.join("A.pbo"),
        &[(
            "config.cpp",
            r#"class CfgPatches { class a_main { requiredAddons[] = {"B_main", "A3_Data_F"}; }; };"#,
        )],
    );
    write_pbo(
        &addons.join("B.pbo"),
        &[("config.cpp", r#"class CfgPatches { class b_main {}; };"#)],
    );
    if let Some(list) = external {
        let list_dir = dir.join(".uksfta");
        fs::create_dir_all(&list_dir).expect("create .uksfta");
        write(&list_dir.join("external-addons.txt"), list.as_bytes());
    }
}

#[test]
fn deps_reports_a_clean_pack() {
    let dir = TempDir::new().expect("temp dir");
    deps_fixture(dir.path(), Some("# base game\nA3_Data_F\n"));

    uksfta(dir.path())
        .arg("deps")
        .assert()
        .success()
        .stdout(predicate::str::contains("provided: 2 addon class(es)"))
        .stdout(predicate::str::contains("requirements: 2"))
        .stdout(predicate::str::contains("unaccounted: 0"));
}

#[test]
fn deps_warns_without_strict_and_fails_with_it() {
    let dir = TempDir::new().expect("temp dir");
    deps_fixture(dir.path(), None);

    uksfta(dir.path())
        .arg("deps")
        .assert()
        .success()
        .stdout(predicate::str::contains("unaccounted: 1"))
        .stdout(predicate::str::contains("A.pbo requires A3_Data_F"));

    uksfta(dir.path())
        .args(["deps", "--strict"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("A.pbo requires A3_Data_F"))
        .stderr(predicate::str::contains("unaccounted"));
}

#[test]
fn deps_suggest_prints_the_external_format_without_writing_it() {
    let dir = TempDir::new().expect("temp dir");
    deps_fixture(dir.path(), None);

    uksfta(dir.path())
        .args(["deps", "--suggest"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".uksfta/external-addons.txt"))
        .stdout(predicate::str::contains("A3_Data_F"));

    assert!(
        !dir.path().join(".uksfta/external-addons.txt").exists(),
        "--suggest must not write the external list"
    );
}

#[test]
fn deps_names_a_missing_build_directory() {
    let dir = TempDir::new().expect("temp dir");
    let missing = dir.path().join("missing-build");

    uksfta(dir.path())
        .args(["deps", "--local"])
        .arg(&missing)
        .assert()
        .code(2)
        .stderr(predicate::str::contains(missing.display().to_string()));
}
