//! End-to-end tests for `uksfta server`. A stub `sftp` script answers the
//! phases from canned fixtures, so no test uses a network or a real server.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

const ROOT: &str = "/srv/arma";
const ID: &str = "3662499431";
const PBO: &str = "uksfta_main.pbo";
const SIG: &str = "uksfta_main.pbo.uksfta_1.0.1.bisign";
const KEY: &str = "uksfta_1.0.1.bikey";

/// A pack that matches the server.
const LISTING_MATCH: &str = "\
sftp> cd \"/srv/arma\"
sftp> -cd \"/srv/arma/@3662499431\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431
sftp> ls -l
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 addons
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 keys
-rw-r--r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
-rw-r--r--    1 1000     1000           11 Nov 20 10:00 mod.cpp
sftp> -cd \"/srv/arma/@3662499431/addons\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/addons
sftp> ls -l
-rw-r--r--    1 1000     1000            5 Nov 20 10:00 uksfta_main.pbo
-rw-r--r--    1 1000     1000            9 Nov 20 10:00 uksfta_main.pbo.uksfta_1.0.1.bisign
sftp> -cd \"/srv/arma/@3662499431/keys\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/keys
sftp> ls -l
-rw-r--r--    1 1000     1000            7 Nov 20 10:00 uksfta_1.0.1.bikey
";

/// The PBO is missing on the server.
const LISTING_MISSING: &str = "\
sftp> cd \"/srv/arma\"
sftp> -cd \"/srv/arma/@3662499431\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431
sftp> ls -l
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 addons
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 keys
-rw-r--r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
-rw-r--r--    1 1000     1000           11 Nov 20 10:00 mod.cpp
sftp> -cd \"/srv/arma/@3662499431/addons\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/addons
sftp> ls -l
-rw-r--r--    1 1000     1000            9 Nov 20 10:00 uksfta_main.pbo.uksfta_1.0.1.bisign
sftp> -cd \"/srv/arma/@3662499431/keys\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/keys
sftp> ls -l
-rw-r--r--    1 1000     1000            7 Nov 20 10:00 uksfta_1.0.1.bikey
";

/// The server name differs only by case.
const LISTING_CASE: &str = "\
sftp> cd \"/srv/arma\"
sftp> -cd \"/srv/arma/@3662499431\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431
sftp> ls -l
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 addons
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 keys
-rw-r--r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
-rw-r--r--    1 1000     1000           11 Nov 20 10:00 mod.cpp
sftp> -cd \"/srv/arma/@3662499431/addons\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/addons
sftp> ls -l
-rw-r--r--    1 1000     1000            5 Nov 20 10:00 UKSFTA_MAIN.PBO
-rw-r--r--    1 1000     1000            9 Nov 20 10:00 uksfta_main.pbo.uksfta_1.0.1.bisign
sftp> -cd \"/srv/arma/@3662499431/keys\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/keys
sftp> ls -l
-rw-r--r--    1 1000     1000            7 Nov 20 10:00 uksfta_1.0.1.bikey
";

/// The pack has no signature on the server.
const LISTING_UNSIGNED: &str = "\
sftp> cd \"/srv/arma\"
sftp> -cd \"/srv/arma/@3662499431\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431
sftp> ls -l
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 addons
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 keys
-rw-r--r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
-rw-r--r--    1 1000     1000           11 Nov 20 10:00 mod.cpp
sftp> -cd \"/srv/arma/@3662499431/addons\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/addons
sftp> ls -l
-rw-r--r--    1 1000     1000            5 Nov 20 10:00 uksfta_main.pbo
sftp> -cd \"/srv/arma/@3662499431/keys\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/keys
sftp> ls -l
-rw-r--r--    1 1000     1000            7 Nov 20 10:00 uksfta_1.0.1.bikey
";

/// A remote-only PBO that `mods.lock` also lists.
const LISTING_EXTRA: &str = "\
sftp> cd \"/srv/arma\"
sftp> -cd \"/srv/arma/@3662499431\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431
sftp> ls -l
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 addons
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 keys
-rw-r--r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
-rw-r--r--    1 1000     1000           11 Nov 20 10:00 mod.cpp
sftp> -cd \"/srv/arma/@3662499431/addons\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/addons
sftp> ls -l
-rw-r--r--    1 1000     1000            5 Nov 20 10:00 uksfta_main.pbo
-rw-r--r--    1 1000     1000            9 Nov 20 10:00 uksfta_main.pbo.uksfta_1.0.1.bisign
-rw-r--r--    1 1000     1000            3 Nov 20 10:00 old.pbo
sftp> -cd \"/srv/arma/@3662499431/keys\"
sftp> pwd
Remote working directory: /srv/arma/@3662499431/keys
sftp> ls -l
-rw-r--r--    1 1000     1000            7 Nov 20 10:00 uksfta_1.0.1.bikey
";

const STUB: &str = r#"#!/bin/sh
set -eu
log="$UKSFTA_TEST_LOG"
printf 'ARGV %s\n' "$*" >> "$log"
commands="$UKSFTA_TEST_STDIN"
cat > "$commands"
printf 'CMDS\n' >> "$log"
cat "$commands" >> "$log"
if grep -q 'ls -l' "$commands"; then
  cat "$UKSFTA_TEST_LISTING"
  exit 0
fi
if grep -q '^put ' "$commands"; then
  marker="${UKSFTA_TEST_PUT_MARKER:-}"
  if [ -n "$marker" ] && [ ! -f "$marker" ]; then
    : > "$marker"
    echo 'fake sftp: connection closed during write' >&2
    exit 1
  fi
fi
if grep -q 'get ' "$commands"; then
  grep 'get ' "$commands" | while read -r command remote local; do
    remote=$(printf '%s' "$remote" | tr -d '"')
    local=$(printf '%s' "$local" | tr -d '"')
    relative=${remote#"$UKSFTA_TEST_ROOT"/}
    if [ -f "$UKSFTA_TEST_REMOTE/$relative" ]; then
      mkdir -p "$(dirname "$local")"
      cp "$UKSFTA_TEST_REMOTE/$relative" "$local"
    fi
  done
fi
exit 0
"#;

/// A temporary pack repo plus the stub's fixtures. No `mod_sources.txt` or
/// `mods.lock` is written, so any read of them would fail the test.
struct Fixture {
    dir: TempDir,
    log: PathBuf,
    stdin: PathBuf,
    stub: PathBuf,
    listing: PathBuf,
    remote: PathBuf,
}

impl Fixture {
    fn new(listing: &str) -> Self {
        let dir = TempDir::new().expect("temp dir");
        let base = dir.path();

        fs::write(
            base.join("meta.cpp"),
            format!("protocol = 1;\npublishedid = {ID};\nname = \"UKSF Taskforce Alpha Maps\";\n"),
        )
        .expect("write meta.cpp");

        let release = base.join(".hemttout/release");
        fs::create_dir_all(release.join("addons")).expect("create addons");
        fs::create_dir_all(release.join("keys")).expect("create keys");
        fs::write(release.join("addons").join(PBO), b"local").expect("write the PBO");
        fs::write(release.join("addons").join(SIG), b"bisign123").expect("write the signature");
        fs::write(release.join("keys").join(KEY), b"bikey12").expect("write the key");
        fs::write(release.join("meta.cpp"), b"meta").expect("write the build meta.cpp");
        fs::write(release.join("mod.cpp"), b"mod-cpp-111").expect("write mod.cpp");

        fs::create_dir_all(base.join(".uksfta")).expect("create the state dir");
        fs::write(
            base.join(".uksfta/server.toml"),
            format!("destination = \"tester@example.invalid\"\nport = 2022\nroot = \"{ROOT}\"\n"),
        )
        .expect("write the config");

        let remote = base.join("remote");
        let pack = remote.join(format!("@{ID}"));
        fs::create_dir_all(&pack).expect("create the remote fixture");
        fs::write(
            pack.join("meta.cpp"),
            format!("protocol = 1;\npublishedid = {ID};\nname = \"UKSF Taskforce Alpha Maps\";\n"),
        )
        .expect("write the remote meta.cpp");
        fs::write(pack.join("mod.cpp"), b"mod-cpp-111").expect("write the remote mod.cpp");

        let listing_path = base.join("listing.txt");
        fs::write(&listing_path, listing).expect("write the listing");
        let log = base.join("sftp.log");
        fs::write(&log, b"").expect("create the log");
        let stdin = base.join("stdin.txt");

        let stub = base.join("fake-sftp");
        fs::write(&stub, STUB).expect("write the stub");
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755))
            .expect("make the stub executable");

        Self {
            dir,
            log,
            stdin,
            stub,
            listing: listing_path,
            remote,
        }
    }

    /// The stub and its fixtures travel in the child environment, so tests
    /// running in parallel cannot interfere.
    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("uksfta").expect("the binary must build");
        command
            .current_dir(self.dir.path())
            .env("UKSFTA_NO_UPDATE_CHECK", "1")
            .env("UKSFTA_SFTP_BIN", &self.stub)
            .env("UKSFTA_TEST_LOG", &self.log)
            .env("UKSFTA_TEST_STDIN", &self.stdin)
            .env("UKSFTA_TEST_LISTING", &self.listing)
            .env("UKSFTA_TEST_REMOTE", &self.remote)
            .env("UKSFTA_TEST_ROOT", ROOT);
        command
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).expect("read the stub log")
    }

    fn write_lock(&self, id: &str, files: &[&str]) {
        let list: Vec<String> = files.iter().map(|file| format!("\"{file}\"")).collect();
        let json = format!(
            "{{\"version\":1,\"mods\":{{\"{id}\":{{\"name\":\"Lock\",\"files\":[{}]}}}}}}",
            list.join(",")
        );
        fs::write(self.dir.path().join("mods.lock"), json).expect("write mods.lock");
    }

    fn write_sources(&self, ids: &[&str]) {
        let mut text = String::from("version = 2\n");
        for id in ids {
            text.push_str(&format!("\n[[mods]]\nid = \"{id}\"\nname = \"Mod {id}\"\n"));
        }
        fs::write(self.dir.path().join("mod_sources.txt"), text).expect("write mod_sources.txt");
    }

    fn remove_local_signature(&self) {
        fs::remove_file(self.dir.path().join(".hemttout/release/addons").join(SIG))
            .expect("remove the local signature");
    }
}

#[test]
fn a_matching_pack_plans_no_change() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture
        .command()
        .arg("server")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "pack 3662499431 UKSF Taskforce Alpha Maps",
        ))
        .stdout(predicate::str::contains("status: ok"))
        .stdout(predicate::str::contains("plan: no changes"));

    let log = fixture.log();
    assert_eq!(
        log.matches("ARGV ").count(),
        2,
        "the listing and the meta phase only"
    );
    let argv: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("ARGV "))
        .collect();
    assert!(
        argv.iter().all(|line| !line.contains(" -b ")),
        "sftp must not run in -b mode: {argv:?}"
    );
    assert!(log.contains("CMDS"), "commands arrive on stdin");
}

#[test]
fn a_missing_pbo_is_reported_and_exits_one() {
    let fixture = Fixture::new(LISTING_MISSING);

    fixture
        .command()
        .arg("server")
        .assert()
        .code(1)
        .stdout(predicate::str::contains("missing uksfta_main.pbo"))
        .stderr(predicate::str::contains("pack problem(s)"));
}

#[test]
fn a_case_only_difference_is_the_same_file() {
    let fixture = Fixture::new(LISTING_CASE);

    fixture
        .command()
        .arg("server")
        .assert()
        .success()
        .stdout(predicate::str::contains("status: ok"))
        .stderr(predicate::str::contains("differ only by case"));

    let log = fixture.log();
    assert!(
        !log.lines().any(|line| line.starts_with("put ")),
        "a case-only difference must not upload"
    );
}

#[test]
fn upload_dry_run_prints_the_plan_and_writes_nothing() {
    let fixture = Fixture::new(LISTING_MISSING);

    fixture
        .command()
        .args(["server", "--upload", "--prune", "--dry-run"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("plan: put"))
        .stdout(predicate::str::contains("dry run: no changes made"));

    let log = fixture.log();
    assert!(
        !log.lines().any(|line| line.starts_with("put ")),
        "a dry run must not upload"
    );
    assert_eq!(
        log.matches("ARGV ").count(),
        2,
        "a dry run must not start an upload phase"
    );
}

#[test]
fn an_unsigned_pack_warns_and_fails_only_when_required() {
    let fixture = Fixture::new(LISTING_UNSIGNED);
    fixture.remove_local_signature();
    fixture
        .command()
        .arg("server")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "unsigned: 1 of 1 PBO(s) have no signature (advisory)",
        ));

    let fixture = Fixture::new(LISTING_UNSIGNED);
    fixture.remove_local_signature();
    fixture
        .command()
        .args(["server", "--require-signatures"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("1 of 1 PBO(s) have no signature"));
}

#[test]
fn a_local_dropin_without_a_meta_cpp_cannot_run() {
    let fixture = Fixture::new(LISTING_MATCH);
    fs::remove_file(fixture.dir.path().join("meta.cpp")).expect("remove meta.cpp");

    fixture
        .command()
        .arg("server")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no meta.cpp"));
}

#[test]
fn a_locked_extra_is_not_pruned_without_force() {
    let fixture = Fixture::new(LISTING_EXTRA);
    fixture.write_lock("1", &["addons/old.pbo"]);

    let assert = fixture
        .command()
        .args(["server", "--prune", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("extra old.pbo"))
        .stderr(predicate::str::contains(
            "old.pbo: listed in mods.lock; not pruned",
        ));
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 output");
    assert!(
        !stdout.contains("plan: rm"),
        "a locked extra is not pruned: {stdout}"
    );

    fixture
        .command()
        .args(["server", "--prune", "--force-prune", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "plan: rm /srv/arma/@3662499431/addons/old.pbo",
        ));
}

#[test]
fn cross_checks_report_a_locked_pbo_and_an_unlocked_source() {
    let fixture = Fixture::new(LISTING_MATCH);
    fixture.write_lock("1111111111", &["addons/absent.pbo"]);
    fixture.write_sources(&["1111111111", "2222222222"]);

    fixture
        .command()
        .arg("server")
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "absent.pbo: locked PBO is not in the local release",
        ))
        .stderr(predicate::str::contains(
            "2222222222: enabled source mod has no mods.lock entry",
        ));
}

#[test]
fn missing_lock_and_sources_stay_silent() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture
        .command()
        .arg("server")
        .assert()
        .success()
        .stderr(predicate::str::contains("mods.lock").not())
        .stderr(predicate::str::contains("source mod").not());
}

#[test]
fn a_failed_upload_is_retried_and_reported() {
    let fixture = Fixture::new(LISTING_MISSING);
    let marker = fixture.dir.path().join("first-put.marker");

    fixture
        .command()
        .env("UKSFTA_TEST_PUT_MARKER", &marker)
        .args(["server", "--upload", "--retries", "1"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("attempt 1: 2 file(s)"))
        .stdout(predicate::str::contains("attempt 2: 2 file(s)"))
        .stdout(predicate::str::contains("MB/s"))
        .stderr(predicate::str::contains("Warning: upload attempt 1"));

    let log = fixture.log();
    assert!(marker.exists(), "the stub must fail the first attempt");
    assert_eq!(
        log.matches("put -p ").count(),
        4,
        "two files per attempt, one restart each, with no resume: {log}"
    );
    assert!(
        !log.contains("reput"),
        "an interrupted upload restarts the file: {log}"
    );
}
