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

/// A public key file with the documented 24-byte header and a zero blob.
fn synthetic_key(authority: &str, public: bool) -> Vec<u8> {
    let bits: u32 = 1024;
    let blob: u32 = if public { 20 + bits / 8 } else { 596 };
    let mut bytes = Vec::new();
    bytes.extend_from_slice(authority.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&blob.to_le_bytes());
    bytes.push(if public { 6 } else { 7 });
    bytes.push(2);
    bytes.extend_from_slice(&[0, 0]);
    bytes.extend_from_slice(&0x2400u32.to_le_bytes());
    bytes.extend_from_slice(if public { b"RSA1" } else { b"RSA2" });
    bytes.extend_from_slice(&bits.to_le_bytes());
    bytes.extend_from_slice(&65537u32.to_le_bytes());
    bytes.resize(bytes.len() + (blob - 20) as usize, 0);
    bytes
}

/// A `.bisign` prefix: the public key then signature bytes this tool ignores.
fn synthetic_signature(authority: &str) -> Vec<u8> {
    let mut bytes = synthetic_key(authority, true);
    bytes.extend_from_slice(&[0, 0, 0, 0]);
    bytes
}

/// A listing for a tree with a nested directory and the usual pack files.
fn parity_listing(python_code: bool) -> String {
    let python_dir = if python_code {
        "drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 python_code\n".to_string()
    } else {
        String::new()
    };
    let python_block = if python_code {
        format!(
            "sftp> -cd \"{ROOT}/@{ID}/python_code\"
sftp> pwd
Remote working directory: {ROOT}/@{ID}/python_code
sftp> ls -l
-rw-r--r--    1 1000     1000            4 Nov 20 10:00 old.py
"
        )
    } else {
        String::new()
    };
    format!(
        "sftp> cd \"{ROOT}\"
sftp> -cd \"{ROOT}/@{ID}\"
sftp> pwd
Remote working directory: {ROOT}/@{ID}
sftp> ls -l
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 addons
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 keys
{python_dir}-rw-r--r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
-rw-r--r--    1 1000     1000           11 Nov 20 10:00 mod.cpp
sftp> -cd \"{ROOT}/@{ID}/addons\"
sftp> pwd
Remote working directory: {ROOT}/@{ID}/addons
sftp> ls -l
-rw-r--r--    1 1000     1000            5 Nov 20 10:00 {PBO}
-rw-r--r--    1 1000     1000            9 Nov 20 10:00 {SIG}
sftp> -cd \"{ROOT}/@{ID}/keys\"
sftp> pwd
Remote working directory: {ROOT}/@{ID}/keys
sftp> ls -l
-rw-r--r--    1 1000     1000            7 Nov 20 10:00 {KEY}
{python_block}"
    )
}

/// A listing for the signing fixtures. Only sizes and names are used.
fn signing_listing(sig_size: usize, key_name: &str, key_size: usize) -> String {
    format!(
        "sftp> cd \"{ROOT}\"
sftp> -cd \"{ROOT}/@{ID}\"
sftp> pwd
Remote working directory: {ROOT}/@{ID}
sftp> ls -l
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 addons
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 keys
-rw-r--r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
-rw-r--r--    1 1000     1000           11 Nov 20 10:00 mod.cpp
sftp> -cd \"{ROOT}/@{ID}/addons\"
sftp> pwd
Remote working directory: {ROOT}/@{ID}/addons
sftp> ls -l
-rw-r--r--    1 1000     1000            5 Nov 20 10:00 {PBO}
-rw-r--r--    1 1000     1000 {sig_size} Nov 20 10:00 {SIG}
sftp> -cd \"{ROOT}/@{ID}/keys\"
sftp> pwd
Remote working directory: {ROOT}/@{ID}/keys
sftp> ls -l
-rw-r--r--    1 1000     1000 {key_size} Nov 20 10:00 {key_name}
"
    )
}

const STUB: &str = r#"#!/bin/sh
set -eu
log="$UKSFTA_TEST_LOG.$$"
commands="$UKSFTA_TEST_STDIN.$$"
printf 'ARGV %s\n' "$*" >> "$log"
cat > "$commands"
printf 'CMDS\n' >> "$log"
cat "$commands" >> "$log"
if [ -n "${UKSFTA_TEST_FAIL_CONNECT:-}" ]; then
  echo 'Connection refused' >&2
  exit 1
fi
if [ -n "${UKSFTA_TEST_HOST_KEY_FAIL:-}" ]; then
  echo 'Host key verification failed.' >&2
  exit 1
fi
if [ -n "${UKSFTA_TEST_KNOWN_HOSTS:-}" ]; then
  printf '%s\n' "host key appended by the stub" >> "$UKSFTA_TEST_KNOWN_HOSTS"
fi
if grep -q 'ls -l' "$commands"; then
  cat "$UKSFTA_TEST_LISTING"
fi
if grep -q '^put ' "$commands"; then
  marker="${UKSFTA_TEST_PUT_MARKER:-}"
  if [ -n "$marker" ] && [ ! -f "$marker" ]; then
    : > "$marker"
    echo 'fake sftp: connection closed during write' >&2
    exit 1
  fi
fi
if ! grep -q 'ls -l' "$commands"; then
  while IFS= read -r line; do
    if [ "$line" = "pwd" ]; then
      printf 'Remote working directory: %s\n' "$UKSFTA_TEST_ROOT"
    fi
  done < "$commands"
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
        let known_hosts = base.join("known_hosts");
        fs::write(&known_hosts, b"fixture key\n").expect("write known_hosts");
        fs::write(
            base.join(".uksfta/server.toml"),
            format!(
                "destination = \"tester@example.invalid\"\nport = 2022\nroot = \"{ROOT}\"\nknown_hosts = \"{}\"\n",
                known_hosts.display()
            ),
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

    /// Every stub invocation writes its own part file, so parallel workers
    /// cannot interleave their output. The parts are read in path order.
    fn log(&self) -> String {
        let prefix = format!("{}.", self.log.display());
        let dir = self.log.parent().expect("the log has a directory");
        let mut parts: Vec<PathBuf> = fs::read_dir(dir)
            .expect("read the fixture directory")
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.to_string_lossy().starts_with(&prefix))
            .collect();
        parts.sort();
        let mut text = String::new();
        for part in parts {
            text.push_str(&fs::read_to_string(part).expect("read a stub log part"));
        }
        text
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

    /// A remote PBO the stub's `get` copies into scratch.
    fn write_remote_pbo(&self, name: &str, body: &[u8]) {
        let addons = self.remote.join(format!("@{ID}/addons"));
        fs::create_dir_all(&addons).expect("create the remote addons");
        fs::write(addons.join(name), body).expect("write the remote PBO");
    }

    /// A config that turns the segmented path on for every file.
    fn enable_segment(&self, min_bytes: u64) {
        let known_hosts = self.dir.path().join("known_hosts");
        fs::write(
            self.dir.path().join(".uksfta/server.toml"),
            format!(
                "destination = \"tester@127.0.0.1\"\nport = 1\nroot = \"{ROOT}\"\nknown_hosts = \"{}\"\nsegment = true\nsegment_min = {min_bytes}\n",
                known_hosts.display()
            ),
        )
        .expect("write the config");
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
        3,
        "the root listing with the meta fetch, the directory listing, and the remote key phase"
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
fn a_deep_run_passes_when_the_remote_copy_matches() {
    let fixture = Fixture::new(&signing_listing(9, KEY, 7));
    fixture.write_remote_pbo(PBO, b"local");

    fixture
        .command()
        .arg("server")
        .arg("--deep")
        .assert()
        .success()
        .stdout(predicate::str::contains("status: ok"))
        .stdout(predicate::str::contains("plan: no changes"))
        // The stub destination has no native session, so this covers the
        // fallback that reads each remote PBO into scratch.
        .stderr(predicate::str::contains("Warning: deep:"));

    assert!(
        fixture
            .log()
            .contains(&format!("-get \"{ROOT}/@{ID}/addons/{PBO}\"")),
        "the fallback downloads the PBO"
    );
}

#[test]
fn a_deep_hash_mismatch_is_reported_and_exits_one() {
    let fixture = Fixture::new(&signing_listing(9, KEY, 7));
    fixture.write_remote_pbo(PBO, b"other");

    fixture
        .command()
        .arg("server")
        .arg("--deep")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "stale uksfta_main.pbo: contents differ",
        ))
        .stderr(predicate::str::contains("pack problem(s)"));
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
        3,
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
        // The second attempt reports success, so the plan needs no re-read.
        .success()
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

#[test]
fn parallel_upload_puts_each_file_exactly_once() {
    let fixture = Fixture::new(LISTING_MISSING);
    fixture
        .command()
        .args(["server", "--upload", "--jobs", "2", "--retries", "0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("attempt 1: 2 file(s)"));

    let log = fixture.log();
    let puts: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("put -p "))
        .collect();
    assert_eq!(puts.len(), 2, "one put per file: {log}");
    assert_eq!(
        puts.iter()
            .filter(|line| line.ends_with("/uksfta_main.pbo\""))
            .count(),
        1,
        "{log}"
    );
    assert_eq!(
        puts.iter()
            .filter(|line| line.ends_with(".bisign\""))
            .count(),
        1,
        "{log}"
    );
    assert!(!log.contains("reput"), "{log}");
}

/// The command block of every client run, in the order the stub logged them.
fn runs(fixture: &Fixture) -> Vec<String> {
    fixture
        .log()
        .split("ARGV ")
        .skip(1)
        .map(str::to_string)
        .collect()
}

fn upload_runs(fixture: &Fixture) -> Vec<String> {
    runs(fixture)
        .into_iter()
        .filter(|run| run.lines().skip(1).any(|line| line.starts_with("put -p ")))
        .collect()
}

#[test]
fn a_serial_upload_with_progress_uses_one_client_for_every_file() {
    let fixture = Fixture::new(LISTING_MISSING);

    fixture
        .command()
        .args([
            "server",
            "--upload",
            "--jobs",
            "1",
            "--progress",
            "--retries",
            "0",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("attempt 1: 2 file(s)"));

    let log = fixture.log();
    assert_eq!(
        log.matches("put -p ").count(),
        2,
        "both files are sent once: {log}"
    );
    let uploads = upload_runs(&fixture);
    assert_eq!(
        uploads.len(),
        1,
        "one client carries every file of the worker: {log}"
    );
    assert_eq!(
        uploads[0].lines().filter(|line| *line == "pwd").count(),
        2,
        "each put is followed by its completion marker: {}",
        uploads[0]
    );
}

#[test]
fn a_clean_upload_does_not_reverify_the_remote_tree() {
    let fixture = Fixture::new(LISTING_MISSING);

    fixture
        .command()
        .args(["server", "--upload", "--jobs", "1", "--retries", "0"])
        .assert()
        .success();

    let runs = runs(&fixture);
    assert_eq!(
        runs.len(),
        4,
        "three read-only phases and one upload: {runs:?}"
    );
    assert!(
        runs.last().is_some_and(|run| run.contains("put -p ")),
        "the upload is the last client run: {runs:?}"
    );
}

/// Every client run uses the error log level, and none passes `-q`.
fn assert_error_log_level(log: &str) {
    let argv: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("ARGV "))
        .collect();
    assert!(!argv.is_empty(), "{log}");
    for line in argv {
        assert!(
            !line.split_whitespace().any(|arg| arg == "-q"),
            "no run may pass -q: {line}"
        );
        assert!(line.contains("-o LogLevel=ERROR"), "{line}");
    }
}

#[test]
fn every_client_run_uses_the_error_log_level() {
    for extra in [Vec::new(), vec!["--progress"]] {
        let fixture = Fixture::new(LISTING_MISSING);
        let mut args = vec!["server", "--upload", "--retries", "0"];
        args.extend(extra);

        fixture.command().args(args).assert().success();

        let log = fixture.log();
        assert!(log.contains("put -p "), "the upload must run: {log}");
        assert_error_log_level(&log);
    }
}

#[test]
fn transfer_flags_reach_every_client_run() {
    let fixture = Fixture::new(LISTING_MISSING);

    fixture
        .command()
        .args([
            "server",
            "--upload",
            "--retries",
            "0",
            "--buffer",
            "261120",
            "--requests",
            "8",
        ])
        .assert()
        .success();

    let log = fixture.log();
    let argv: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("ARGV "))
        .collect();
    assert!(!argv.is_empty(), "{log}");
    assert!(
        argv.iter()
            .all(|line| line.contains("-B 261120") && line.contains("-R 8")),
        "every phase must use the configured transfer values: {argv:?}"
    );
}

#[test]
fn an_out_of_range_transfer_value_fails_before_any_connection() {
    for args in [
        ["server", "--buffer", "262144"],
        ["server", "--buffer", "0"],
        ["server", "--requests", "257"],
        ["server", "--requests", "0"],
    ] {
        let fixture = Fixture::new(LISTING_MATCH);

        fixture
            .command()
            .args(args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("must be"));

        assert!(
            !fixture.log().contains("ARGV "),
            "no client may start for {args:?}"
        );
    }
}

#[test]
fn accept_new_adds_the_key_and_prints_a_notice() {
    let fixture = Fixture::new(LISTING_MATCH);
    let known_hosts = fixture.dir.path().join("accepted_known_hosts");

    fixture
        .command()
        .args(["server", "--host-key-policy", "accept-new", "--known-hosts"])
        .arg(&known_hosts)
        .env("UKSFTA_TEST_KNOWN_HOSTS", &known_hosts)
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "host key for tester@example.invalid added to",
        ));

    assert!(known_hosts.exists(), "the stub must add the key");
}

#[test]
fn a_failed_host_key_check_names_the_host_and_the_file() {
    let fixture = Fixture::new(LISTING_MATCH);
    let known_hosts = fixture.dir.path().join("empty_known_hosts");
    fs::write(&known_hosts, b"").expect("write an empty known_hosts");

    fixture
        .command()
        .args(["server", "--known-hosts"])
        .arg(&known_hosts)
        .env("UKSFTA_TEST_HOST_KEY_FAIL", "1")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("host key check failed"))
        .stderr(predicate::str::contains("tester@example.invalid"))
        .stderr(predicate::str::contains(known_hosts.display().to_string()))
        .stderr(predicate::str::contains("--host-key-policy accept-new"));
}

#[test]
fn a_missing_known_hosts_stops_strict_mode_before_any_client() {
    let fixture = Fixture::new(LISTING_MATCH);
    let known_hosts = fixture.dir.path().join("absent_known_hosts");

    fixture
        .command()
        .args(["server", "--known-hosts"])
        .arg(&known_hosts)
        .assert()
        .code(2)
        .stderr(predicate::str::contains(known_hosts.display().to_string()))
        .stderr(predicate::str::contains("--host-key-policy accept-new"));

    assert!(
        !fixture.log().contains("ARGV "),
        "no client may start without a readable known_hosts"
    );
}

#[test]
fn an_unknown_host_key_policy_is_refused() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture
        .command()
        .args(["server", "--host-key-policy", "maybe"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "host key policy must be yes or accept-new",
        ));

    assert!(!fixture.log().contains("ARGV "));
}

/// A pack with a synthetic signature and one renamed, synthetic key.
fn signing_fixture(signature_authority: &str, key_authority: &str) -> Fixture {
    let signature = synthetic_signature(signature_authority);
    let key = synthetic_key(key_authority, true);
    let listing = signing_listing(signature.len(), "renamed.bikey", key.len());
    let fixture = Fixture::new(&listing);

    let release = fixture.dir.path().join(".hemttout/release");
    fs::write(release.join("addons").join(SIG), &signature).expect("write the signature");
    fs::remove_file(release.join("keys").join(KEY)).expect("remove the old key");
    fs::write(release.join("keys").join("renamed.bikey"), &key).expect("write the local key");

    let pack = fixture.remote.join(format!("@{ID}"));
    fs::create_dir_all(pack.join("keys")).expect("create the remote keys dir");
    fs::write(pack.join("keys").join("renamed.bikey"), &key).expect("write the remote key");
    fixture
}

#[test]
fn a_renamed_key_with_the_same_authority_passes() {
    let fixture = signing_fixture("uksfta_1.0.1", "uksfta_1.0.1");

    fixture
        .command()
        .arg("server")
        .assert()
        .success()
        .stdout(predicate::str::contains("status: ok"))
        .stderr(predicate::str::contains(
            "renamed.bikey: embedded authority is uksfta_1.0.1",
        ));

    let log = fixture.log();
    assert!(
        log.contains(&format!("-get \"{ROOT}/@{ID}/keys/renamed.bikey\"")),
        "the remote key is read: {log}"
    );
    assert!(
        !log.lines()
            .any(|line| line.starts_with("-get ") && line.contains(".bisign")),
        "a remote signature is never downloaded: {log}"
    );
}

#[test]
fn a_signature_authority_without_a_key_exits_one() {
    let fixture = signing_fixture("other_auth_1", "uksfta_1.0.1");

    fixture
        .command()
        .arg("server")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(format!(
            "signature {SIG} needs key other_auth_1, which keys/ does not ship"
        )));
}

/// A release tree with a root file, a nested file and three exclusions.
fn parity_fixture(listing: &str) -> Fixture {
    let fixture = Fixture::new(listing);
    let release = fixture.dir.path().join(".hemttout/release");
    fs::write(release.join("LICENSE"), b"licence").expect("write LICENSE");
    fs::create_dir_all(release.join("python_code")).expect("create python_code");
    fs::write(release.join("python_code/x.py"), b"py!").expect("write x.py");
    fs::write(release.join("sign.biprivatekey"), b"priv").expect("write the private key");
    fs::write(release.join(".DS_Store"), b"ds").expect("write the dotfile");
    fs::write(release.join("x.tmp"), b"tmp").expect("write the temp file");

    let pack = fixture.remote.join(format!("@{ID}"));
    fs::create_dir_all(pack.join("python_code")).expect("create the remote nested dir");
    fs::write(pack.join("python_code/old.py"), b"old!").expect("write the remote extra");
    fixture
}

#[test]
fn the_whole_release_root_is_compared() {
    let fixture = parity_fixture(&parity_listing(true));

    let assert = fixture
        .command()
        .args(["server", "--upload", "--prune", "--dry-run"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("missing LICENSE"))
        .stdout(predicate::str::contains("missing python_code/x.py"))
        .stdout(predicate::str::contains("extra python_code/old.py"))
        .stderr(predicate::str::contains("release file(s) are excluded"));

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf-8 output");
    assert!(
        stdout.contains(&format!("plan: put {ROOT}/@{ID}/LICENSE")),
        "{stdout}"
    );
    assert!(
        stdout.contains(&format!("plan: put {ROOT}/@{ID}/python_code/x.py")),
        "{stdout}"
    );
    assert!(!stdout.contains("plan: rm"), "no nested file is pruned");
    for excluded in ["biprivatekey", ".DS_Store", "x.tmp"] {
        assert!(!stdout.contains(excluded), "{excluded} must be excluded");
    }
}

#[test]
fn a_nested_upload_creates_the_directory_first() {
    let fixture = parity_fixture(&parity_listing(true));

    fixture
        .command()
        .args(["server", "--upload", "--prune", "--retries", "0"])
        .assert()
        // A clean batch ends the run, so the leftover remote extra decides
        // the status but not a failed upload.
        .success();

    let log = fixture.log();
    let release = std::path::Path::new(".hemttout/release");
    let mkdir = log
        .find(&format!("-mkdir \"{ROOT}/@{ID}/python_code\""))
        .unwrap_or_else(|| panic!("mkdir missing: {log}"));
    let put = log
        .find(&format!(
            "put -p \"{}\"",
            release.join("python_code/x.py").display()
        ))
        .expect("the nested file is sent");
    assert!(mkdir < put, "the directory comes before its file: {log}");
    assert!(
        log.contains(&format!("put -p \"{}\"", release.join("LICENSE").display())),
        "{log}"
    );
    for excluded in ["biprivatekey", ".DS_Store", "x.tmp"] {
        assert!(!log.contains(excluded), "{excluded} must be excluded");
    }
    assert!(
        !log.lines().any(|line| line.starts_with("rm ")),
        "a nested extra is never removed: {log}"
    );
}

#[test]
fn an_unlisted_remote_directory_is_empty_not_an_error() {
    let fixture = parity_fixture(&parity_listing(false));

    fixture
        .command()
        .args(["server", "--dry-run"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("missing LICENSE"))
        .stdout(predicate::str::contains("missing python_code/x.py"));
}

#[test]
fn a_symlinked_remote_directory_is_refused() {
    let listing = format!(
        "sftp> cd \"{ROOT}\"
sftp> -cd \"{ROOT}/@{ID}\"
sftp> pwd
Remote working directory: {ROOT}/@{ID}
sftp> ls -l
lrwxrwxrwx    1 1000     1000            9 Nov 20 10:00 addons -> /elsewhere
drwxr-xr-x    2 1000     1000         4096 Nov 20 10:00 keys
-rw-r--r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
"
    );
    let fixture = Fixture::new(&listing);

    fixture
        .command()
        .arg("server")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "addons is a symlink; refusing to follow it",
        ));
}

/// Write a remote `server.cfg` at the given path below the server root.
fn write_remote_cfg(fixture: &Fixture, rel: &str, text: &str) {
    let path = fixture.remote.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create the cfg directory");
    }
    fs::write(path, text).expect("write the remote config");
}

/// A local launch-line file in the fixture directory.
fn write_launch_line(fixture: &Fixture, line: &str) -> PathBuf {
    let path = fixture.dir.path().join("launch.txt");
    fs::write(&path, line).expect("write the launch line");
    path
}

#[test]
fn a_launch_line_that_names_the_pack_passes() {
    let fixture = Fixture::new(LISTING_MATCH);
    let line = write_launch_line(&fixture, "-mod=@3662499431");

    fixture
        .command()
        .args(["server", "--launch-line"])
        .arg(&line)
        .assert()
        .success()
        .stderr(predicate::str::contains("launch: @3662499431 is loaded"));
}

#[test]
fn a_server_mod_entry_reports_not_in_mod() {
    let fixture = Fixture::new(LISTING_MATCH);
    let line = write_launch_line(&fixture, "-serverMod=mods/@3662499431");

    fixture
        .command()
        .args(["server", "--launch-line"])
        .arg(&line)
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "launch: @3662499431 is not in -mod",
        ));
}

#[test]
fn an_absent_pack_is_a_warning_unless_strict() {
    let fixture = Fixture::new(LISTING_MATCH);
    let line = write_launch_line(&fixture, "-mod=@999");

    fixture
        .command()
        .args(["server", "--launch-line"])
        .arg(&line)
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "launch: @3662499431 is not loaded",
        ));

    fixture
        .command()
        .args(["server", "--launch-line"])
        .arg(&line)
        .arg("--strict-config")
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "launch: @3662499431 is not loaded",
        ))
        .stderr(predicate::str::contains("1 pack problem").not());
}

#[test]
fn a_par_file_hides_the_list() {
    let fixture = Fixture::new(LISTING_MATCH);
    let line = write_launch_line(&fixture, "-par=server.par -mod=@3662499431");

    fixture
        .command()
        .args(["server", "--launch-line"])
        .arg(&line)
        .arg("--strict-config")
        .assert()
        .success()
        .stderr(predicate::str::contains("launch: -par hides the mod list"));
}

#[test]
fn the_pack_in_both_lists_is_loaded_twice() {
    let fixture = Fixture::new(LISTING_MATCH);
    let line = write_launch_line(&fixture, "-mod=@3662499431 -serverMod=@3662499431");

    fixture
        .command()
        .args(["server", "--launch-line"])
        .arg(&line)
        .assert()
        .success()
        .stderr(predicate::str::contains("launch: loaded twice"));

    fixture
        .command()
        .args(["server", "--launch-line"])
        .arg(&line)
        .arg("--strict-config")
        .assert()
        .code(1)
        .stderr(predicate::str::contains("launch: loaded twice"));
}

#[test]
fn a_missing_launch_line_file_stops_before_any_client() {
    let fixture = Fixture::new(LISTING_MATCH);
    let missing = fixture.dir.path().join("absent-launch.txt");

    fixture
        .command()
        .args(["server", "--launch-line"])
        .arg(&missing)
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no launch line at"));

    assert!(!fixture.log().contains("ARGV "));
}

#[test]
fn a_correct_server_cfg_passes_and_is_fetched_once() {
    let fixture = Fixture::new(LISTING_MATCH);
    write_remote_cfg(
        &fixture,
        "server.cfg",
        "verifySignatures = 2;\nallowedFilePatching = 0;\n",
    );

    fixture
        .command()
        .args(["server", "--server-cfg", "server.cfg", "--strict-config"])
        .assert()
        .success()
        .stderr(predicate::str::contains("server.cfg: verifySignatures = 2"))
        .stderr(predicate::str::contains(
            "server.cfg: allowedFilePatching = 0",
        ));

    let log = fixture.log();
    assert_eq!(
        log.lines()
            .filter(|line| line.starts_with("-get ") && line.contains("server.cfg"))
            .count(),
        1,
        "one get for the config: {log}"
    );
}

#[test]
fn an_unsafe_server_cfg_fails_only_with_strict() {
    let fixture = Fixture::new(LISTING_MATCH);
    write_remote_cfg(
        &fixture,
        "server.cfg",
        "verifySignatures = 0;\nallowedFilePatching = 2;\n",
    );

    fixture
        .command()
        .args(["server", "--server-cfg", "server.cfg"])
        .assert()
        .success()
        .stderr(predicate::str::contains("server.cfg: verifySignatures = 0"))
        .stderr(predicate::str::contains(
            "server.cfg: allowedFilePatching = 2",
        ));

    fixture
        .command()
        .args(["server", "--server-cfg", "server.cfg", "--strict-config"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("server.cfg: verifySignatures = 0"))
        .stderr(predicate::str::contains(
            "server.cfg: allowedFilePatching = 2",
        ));
}

#[test]
fn a_comment_or_a_string_cannot_hide_the_setting() {
    let fixture = Fixture::new(LISTING_MATCH);
    write_remote_cfg(
        &fixture,
        "server.cfg",
        "hostname = \"verifySignatures = 0;\";\n// verifySignatures = 0;\nverifySignatures = 2;\n",
    );

    fixture
        .command()
        .args(["server", "--server-cfg", "server.cfg", "--strict-config"])
        .assert()
        .success()
        .stderr(predicate::str::contains("server.cfg: verifySignatures = 2"))
        .stderr(predicate::str::contains("server.cfg: verifySignatures = 0").not());
}

#[test]
fn file_patching_exceptions_warn_but_do_not_fail_strict() {
    let fixture = Fixture::new(LISTING_MATCH);
    write_remote_cfg(
        &fixture,
        "server.cfg",
        "verifySignatures = 2;\nallowedFilePatching = 0;\nfilePatchingExceptions[] = { \"7656119\" };\n",
    );

    fixture
        .command()
        .args(["server", "--server-cfg", "server.cfg", "--strict-config"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "server.cfg: filePatchingExceptions is not empty",
        ));
}

#[test]
fn the_config_is_not_fetched_without_the_flag() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture.command().arg("server").assert().success();

    assert!(
        !fixture.log().contains("server.cfg"),
        "no config is fetched: {}",
        fixture.log()
    );
}

/// Parse the one JSON object on stdout.
fn json_stdout(output: &std::process::Output) -> serde_json::Value {
    let text = String::from_utf8(output.stdout.clone()).expect("utf-8 output");
    assert_eq!(
        text.lines().count(),
        1,
        "standard output holds one JSON line: {text}"
    );
    serde_json::from_str(&text).expect("valid JSON")
}

#[test]
fn json_reports_a_matching_pack() {
    let fixture = Fixture::new(LISTING_MATCH);

    let assert = fixture
        .command()
        .args(["server", "--json"])
        .assert()
        .success();
    let value = json_stdout(assert.get_output());

    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["result"], "ok");
    assert_eq!(value["status"], "ok");
    assert_eq!(value["pack"]["id"], ID);
    assert_eq!(value["server"]["port"], 2022);
    assert_eq!(value["server"]["buffer"], 32768);
    assert_eq!(value["server"]["requests"], 64);
    assert_eq!(value["counts"]["local_pbos"], 1);
    assert!(value["timings"]["connect_ms"].as_u64().is_some());
    assert!(value["timings"]["total_ms"].as_u64().is_some());
    assert!(value["findings"].is_array());
    assert!(value["actions"].is_array());
}

#[test]
fn json_reports_a_missing_pack_as_check_failed() {
    let fixture = Fixture::new(LISTING_MISSING);

    let assert = fixture
        .command()
        .args(["server", "--json"])
        .assert()
        .code(1);
    let value = json_stdout(assert.get_output());

    assert_eq!(value["result"], "check-failed");
    assert_eq!(value["status"], "missing");
    assert!(value["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .any(|finding| finding["kind"] == "missing" && finding["name"] == "uksfta_main.pbo"));
}

#[test]
fn json_survives_an_input_error() {
    let fixture = Fixture::new(LISTING_MATCH);
    fs::remove_file(fixture.dir.path().join("meta.cpp")).expect("remove meta.cpp");

    let assert = fixture
        .command()
        .args(["server", "--json"])
        .assert()
        .code(2);
    let value = json_stdout(assert.get_output());

    assert_eq!(value["result"], "input-error");
    assert!(value["error"]
        .as_str()
        .is_some_and(|text| text.contains("meta.cpp")));
}

#[test]
fn doctor_reports_every_effective_setting() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture
        .command()
        .args(["server", "--doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "doctor: destination tester@example.invalid",
        ))
        .stdout(predicate::str::contains("doctor: port 2022"))
        .stdout(predicate::str::contains("doctor: root /srv/arma"))
        .stdout(predicate::str::contains("doctor: known_hosts"))
        .stdout(predicate::str::contains("doctor: host_key_policy yes"))
        .stdout(predicate::str::contains("doctor: buffer 32768"))
        .stdout(predicate::str::contains("doctor: requests 64"))
        .stdout(predicate::str::contains("doctor: connect ok"))
        .stdout(predicate::str::contains("doctor: pack ok"))
        .stdout(predicate::str::contains("doctor: free space unknown"));
}

#[test]
fn doctor_reports_an_unreachable_server_and_exits_two() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture
        .command()
        .args(["server", "--doctor"])
        .env("UKSFTA_TEST_FAIL_CONNECT", "1")
        .assert()
        .code(2)
        .stdout(predicate::str::contains("doctor: connect failed"))
        .stdout(predicate::str::contains("doctor: buffer 32768"));
}

#[test]
fn doctor_works_with_json() {
    let fixture = Fixture::new(LISTING_MATCH);

    let assert = fixture
        .command()
        .args(["server", "--doctor", "--json"])
        .assert()
        .success();
    let value = json_stdout(assert.get_output());

    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["mode"], "doctor");
    assert_eq!(value["result"], "ok");
    assert_eq!(value["server"]["buffer"], 32768);
    assert!(value["free_space_bytes"].is_null());
    assert!(value["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .any(|check| check["name"] == "connect" && check["ok"] == true));
    assert!(value["timings"]["connect_ms"].as_u64().is_some());
}

#[test]
fn human_runs_print_one_timing_line() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture
        .command()
        .arg("server")
        .assert()
        .success()
        .stderr(predicate::str::contains("timing: connect/list"))
        .stderr(predicate::str::contains("total"));
}

#[test]
fn a_small_file_is_uploaded_the_same_with_and_without_the_flag() {
    let plain = Fixture::new(LISTING_MISSING);
    let flagged = Fixture::new(LISTING_MISSING);

    let plain_out = plain
        .command()
        .args(["server", "--upload", "--retries", "0"])
        .output()
        .expect("run without the flag");
    let flagged_out = flagged
        .command()
        .args(["server", "--upload", "--retries", "0", "--segmented"])
        .output()
        .expect("run with the flag");

    assert_eq!(plain_out.status.code(), flagged_out.status.code());
    assert_eq!(
        stable_stdout(&String::from_utf8_lossy(&plain_out.stdout)),
        stable_stdout(&String::from_utf8_lossy(&flagged_out.stdout)),
        "a file below the minimum must not change stdout"
    );
    assert_eq!(puts(&plain), puts(&flagged), "the same put lines");
    assert_eq!(puts(&plain), 2, "both files use the system path");
}

/// Standard output with the elapsed and rate fields of an attempt line
/// replaced, so two runs can be compared byte for byte.
fn stable_stdout(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.starts_with("attempt ") {
                let head = line.split_once(':').map(|(head, _)| head).unwrap_or(line);
                format!("{head}: <timing>")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn puts(fixture: &Fixture) -> usize {
    fixture
        .log()
        .lines()
        .filter(|line| line.starts_with("put -p "))
        .count()
}

#[test]
fn a_config_that_enables_segment_prints_the_prediction_without_a_connection() {
    let fixture = Fixture::new(LISTING_MISSING);
    fixture.enable_segment(1);

    fixture
        .command()
        .args(["server", "--upload", "--dry-run"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("segment: uksfta_main.pbo"));

    assert_eq!(puts(&fixture), 0, "a dry run sends nothing");
}

#[test]
fn the_legacy_flag_overrides_a_config_that_enables_segment() {
    let fixture = Fixture::new(LISTING_MISSING);
    fixture.enable_segment(1);

    fixture
        .command()
        .args(["server", "--upload", "--retries", "0", "--legacy-sftp"])
        .assert()
        .success()
        .stdout(predicate::str::contains("attempt 1: 2 file(s)"))
        .stdout(predicate::str::contains("segment:").not());

    assert_eq!(puts(&fixture), 2, "the system path sends both files");
}

#[test]
fn the_two_segment_flags_conflict() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture
        .command()
        .args(["server", "--segmented", "--legacy-sftp"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn json_reports_the_segment_settings_only_when_requested() {
    let plain = Fixture::new(LISTING_MATCH);
    let assert = plain
        .command()
        .args(["server", "--json"])
        .assert()
        .success();
    let value = json_stdout(assert.get_output());
    assert!(
        value["server"].get("segment").is_none(),
        "absent by default"
    );
    assert!(value.get("uploads").is_none(), "absent by default");

    let flagged = Fixture::new(LISTING_MATCH);
    let assert = flagged
        .command()
        .args(["server", "--json", "--segmented"])
        .assert()
        .success();
    let value = json_stdout(assert.get_output());
    assert_eq!(value["server"]["segment"]["enabled"], true);
    assert_eq!(value["server"]["segment"]["min_bytes"], 67_108_864u64);
    assert_eq!(value["server"]["segment"]["connections"], 4);
    assert_eq!(value["server"]["segment"]["chunk"], serde_json::Value::Null);
    assert_eq!(value["server"]["segment"]["resume"], "auto");
    assert_eq!(value["server"]["segment"]["reconnects"], 3);
    assert!(value["uploads"].is_array(), "the upload list is present");
}

#[test]
fn the_doctor_reports_the_segment_settings_only_when_requested() {
    let plain = Fixture::new(LISTING_MATCH);
    plain
        .command()
        .args(["server", "--doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("segment: not requested"));

    let flagged = Fixture::new(LISTING_MATCH);
    flagged.enable_segment(1);
    flagged
        .command()
        .args(["server", "--doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "segment: min 1 bytes, 4 connections, chunk auto, resume auto, 3 reconnects",
        ))
        .stdout(predicate::str::contains("segment: largest "))
        .stdout(predicate::str::contains("segment: server check failed:"));
}

#[test]
fn a_dry_run_marks_the_selected_file_as_segmented() {
    let fixture = Fixture::new(LISTING_MISSING);

    let assert = fixture
        .command()
        .args([
            "server",
            "--json",
            "--dry-run",
            "--segmented",
            "--segment-min",
            "1",
        ])
        .assert()
        .code(1);
    let value = json_stdout(assert.get_output());
    println!("JSON:{}", serde_json::to_string(&value).expect("json"));

    let uploads = value["uploads"].as_array().expect("an upload list");
    let segmented: Vec<&serde_json::Value> = uploads
        .iter()
        .filter(|upload| upload["mode"] == "segmented")
        .collect();
    assert_eq!(
        segmented.len(),
        1,
        "exactly one planned target: {uploads:?}"
    );
    assert_eq!(segmented[0]["connections"], 4);
    assert!(
        segmented[0].get("sha256").is_none(),
        "no hash before publication"
    );

    let largest = value["actions"]
        .as_array()
        .expect("actions")
        .iter()
        .filter(|action| action["kind"] == "upload")
        .max_by_key(|action| action["size"].as_u64().unwrap_or_default())
        .expect("an upload action");
    assert_eq!(
        segmented[0]["name"], largest["name"],
        "the segmented entry is the largest planned file"
    );

    let plain = Fixture::new(LISTING_MISSING);
    let assert = plain
        .command()
        .args(["server", "--json", "--dry-run"])
        .assert()
        .code(1);
    let value = json_stdout(assert.get_output());
    assert!(value["server"].get("segment").is_none());
    assert!(value.get("uploads").is_none());
}

#[test]
fn the_password_prompt_refuses_without_a_terminal() {
    let fixture = Fixture::new(LISTING_MATCH);
    let output = std::process::Command::new("setsid")
        .arg("-w")
        .arg(env!("CARGO_BIN_EXE_uksfta"))
        .args(["server", "--password-prompt"])
        .current_dir(fixture.dir.path())
        .env("UKSFTA_NO_UPDATE_CHECK", "1")
        .env("UKSFTA_SFTP_BIN", &fixture.stub)
        .env("UKSFTA_TEST_LOG", &fixture.log)
        .env("UKSFTA_TEST_STDIN", &fixture.stdin)
        .env("UKSFTA_TEST_LISTING", &fixture.listing)
        .env("UKSFTA_TEST_REMOTE", &fixture.remote)
        .env("UKSFTA_TEST_ROOT", ROOT)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("setsid");

    assert_eq!(output.status.code(), Some(2));
    let errors = String::from_utf8_lossy(&output.stderr);
    for name in ["--identity", "ssh-agent", "sftp"] {
        assert!(
            errors.contains(name),
            "the message must name {name}: {errors}"
        );
    }
    assert_eq!(fixture.log(), "", "no sftp invocation is made");
}

#[test]
fn the_password_prompt_conflicts_with_the_legacy_switch() {
    let fixture = Fixture::new(LISTING_MATCH);

    fixture
        .command()
        .args(["server", "--password-prompt", "--legacy-sftp"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn a_password_prompt_config_key_is_refused() {
    let fixture = Fixture::new(LISTING_MATCH);
    let path = fixture.dir.path().join(".uksfta/server.toml");
    let mut text = fs::read_to_string(&path).expect("read the config");
    text.push_str("password_prompt = true\n");
    fs::write(&path, text).expect("write the config");

    fixture
        .command()
        .arg("server")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("password_prompt"));
}
