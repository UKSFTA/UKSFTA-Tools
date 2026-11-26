//! End-to-end tests for `uksfta server`. A stub `sftp` script answers the
//! batch phases from canned fixtures, so no test uses a network or a real
//! server.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

const ROOT: &str = "/srv/arma";

/// A mod with a PBO present and signed, and a mod whose PBO is missing.
const LISTING: &str = "\
sftp> cd \"/srv/arma\"
sftp> pwd
Remote working directory: /srv/arma
sftp> ls -l
drwxrwxr-x    3 1000     1000         4096 Nov 20 10:00 @450814997
drwxrwxr-x    3 1000     1000         4096 Nov 20 10:00 @887302721
drwxrwxr-x    2 1000     1000         4096 Nov 20 10:00 addons
-rw-rw-r--    1 1000     1000         1234 Nov 20 10:00 server.cfg
sftp> -cd \"/srv/arma/@450814997\"
sftp> pwd
Remote working directory: /srv/arma/@450814997
sftp> ls -l
drwxrwxr-x    2 1000     1000         4096 Nov 20 10:00 addons
-rw-rw-r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
sftp> -cd \"/srv/arma/@450814997/addons\"
sftp> pwd
Remote working directory: /srv/arma/@450814997/addons
sftp> ls -l
-rw-rw-r--    1 1000     1000            5 Nov 20 10:00 CBA.pbo
-rw-rw-r--    1 1000     1000           10 Nov 20 10:00 CBA.pbo.cba_3.20.0.bisign
sftp> -cd \"/srv/arma/@887302721\"
sftp> pwd
Remote working directory: /srv/arma/@887302721
sftp> ls -l
drwxrwxr-x    2 1000     1000         4096 Nov 20 10:00 addons
-rw-rw-r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
sftp> -cd \"/srv/arma/@887302721/addons\"
sftp> pwd
Remote working directory: /srv/arma/@887302721/addons
sftp> ls -l
";

/// One mod whose PBO is present but has no companion signature.
const LISTING_UNSIGNED: &str = "\
sftp> cd \"/srv/arma\"
sftp> pwd
Remote working directory: /srv/arma
sftp> ls -l
drwxrwxr-x    3 1000     1000         4096 Nov 20 10:00 @450814997
sftp> -cd \"/srv/arma/@450814997\"
sftp> pwd
Remote working directory: /srv/arma/@450814997
sftp> ls -l
drwxrwxr-x    2 1000     1000         4096 Nov 20 10:00 addons
-rw-rw-r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
sftp> -cd \"/srv/arma/@450814997/addons\"
sftp> pwd
Remote working directory: /srv/arma/@450814997/addons
sftp> ls -l
-rw-rw-r--    1 1000     1000            5 Nov 20 10:00 CBA.pbo
";

/// One mod whose `addons/` directory is a symlink.
const LISTING_SYMLINK: &str = "\
sftp> cd \"/srv/arma\"
sftp> pwd
Remote working directory: /srv/arma
sftp> ls -l
drwxrwxr-x    3 1000     1000         4096 Nov 20 10:00 @450814997
sftp> -cd \"/srv/arma/@450814997\"
sftp> pwd
Remote working directory: /srv/arma/@450814997
sftp> ls -l
lrwxrwxrwx    1 1000     1000            9 Nov 20 10:00 addons -> /srv/other
-rw-rw-r--    1 1000     1000          120 Nov 20 10:00 meta.cpp
sftp> -cd \"/srv/arma/@450814997/addons\"
sftp> pwd
Remote working directory: /srv/other
sftp> ls -l
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

/// A temporary mod repository plus the stub's fixtures.
struct Fixture {
    dir: TempDir,
    log: PathBuf,
    stub: PathBuf,
    listing: PathBuf,
    remote: PathBuf,
    cache: PathBuf,
    stdin: PathBuf,
}

impl Fixture {
    fn new(mods: &[(&str, &str, &str)], listing: &str) -> Self {
        let dir = TempDir::new().expect("temp dir");
        let base = dir.path();

        let mut sources = String::from("version = 2\n");
        let mut lock = String::from("{\"version\":1,\"mods\":{");
        for (index, (id, name, pbo)) in mods.iter().enumerate() {
            sources.push_str(&format!("\n[[mods]]\nid = \"{id}\"\nname = \"{name}\"\n"));
            if index > 0 {
                lock.push(',');
            }
            lock.push_str(&format!(
                "\"{id}\":{{\"name\":\"{name}\",\"files\":[\"addons/{pbo}\"]}}"
            ));
            fs::create_dir_all(base.join("addons")).expect("create addons");
            fs::write(base.join("addons").join(pbo), b"local").expect("write a local PBO");
        }
        lock.push_str("}}");

        fs::create_dir_all(base.join(".uksfta")).expect("create the state dir");
        fs::write(
            base.join(".uksfta/server.toml"),
            format!("destination = \"tester@example.invalid\"\nport = 2022\nroot = \"{ROOT}\"\n"),
        )
        .expect("write the config");
        fs::write(base.join("mod_sources.txt"), sources).expect("write mod_sources.txt");
        fs::write(base.join("mods.lock"), lock).expect("write mods.lock");

        let remote = base.join("remote");
        let cache = base.join("cache");
        for (id, _, pbo) in mods {
            fs::create_dir_all(remote.join(format!("@{id}"))).expect("create the remote fixture");
            fs::write(
                remote.join(format!("@{id}/meta.cpp")),
                format!("protocol = 1;\npublishedid = {id};\nname = \"Fixture\";\n"),
            )
            .expect("write the remote meta.cpp");
            fs::create_dir_all(cache.join(format!("{id}/addons"))).expect("create the cache");
            fs::write(
                cache.join(format!("{id}/meta.cpp")),
                format!("protocol = 1;\npublishedid = {id};\nname = \"Fixture\";\n"),
            )
            .expect("write the cached meta.cpp");
            let signature = format!("{}/addons/{pbo}.key_1.0.bisign", cache.join(id).display());
            fs::write(signature, b"signature").expect("write a cached signature");
            // The cache PBO version must match the local PBO for the
            // signature to pair with it.
            fs::write(cache.join(id).join("addons").join(pbo), b"local")
                .expect("write the cached PBO");
        }
        // The listing names this signature, so the sizes must agree.
        fs::write(
            cache.join("450814997/addons/CBA.pbo.cba_3.20.0.bisign"),
            b"0123456789",
        )
        .expect("write the signed CBA signature");

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
            stub,
            listing: listing_path,
            remote,
            cache,
            stdin,
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
            .env("UKSFTA_TEST_ROOT", ROOT)
            .env("UKSFTA_WORKSHOP_CACHE", &self.cache);
        command
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log).expect("read the stub log")
    }
}

#[test]
fn a_missing_pbo_is_reported_and_exits_one() {
    let fixture = Fixture::new(
        &[
            ("450814997", "CBA_A3", "CBA.pbo"),
            ("887302721", "Boat Mod", "Boat.pbo"),
        ],
        LISTING,
    );

    fixture
        .command()
        .arg("server")
        .assert()
        .code(1)
        .stdout(predicate::str::contains(
            "mod 450814997 CBA_A3: ok (1 PBO(s))",
        ))
        .stdout(predicate::str::contains("missing Boat.pbo"))
        .stderr(predicate::str::contains("server problem(s) found"));

    let log = fixture.log();
    assert_eq!(
        log.matches("ARGV ").count(),
        2,
        "a report run uses the listing and meta phases only"
    );
    let argv: Vec<&str> = log
        .lines()
        .filter(|line| line.starts_with("ARGV "))
        .collect();
    assert!(
        argv.iter().all(|line| !line.contains(" -b ")),
        "sftp must not run in -b mode: {argv:?}"
    );
    assert!(
        log.contains("CMDS"),
        "the stub must receive the commands on stdin"
    );
}

#[test]
fn upload_dry_run_prints_the_plan_and_writes_nothing() {
    let fixture = Fixture::new(
        &[
            ("450814997", "CBA_A3", "CBA.pbo"),
            ("887302721", "Boat Mod", "Boat.pbo"),
        ],
        LISTING,
    );

    fixture
        .command()
        .args(["server", "--upload", "--dry-run"])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("plan: put"))
        .stdout(predicate::str::contains("Boat.pbo"))
        .stdout(predicate::str::contains("Boat.pbo.key_1.0.bisign"))
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
fn a_symlinked_addons_directory_is_refused() {
    let fixture = Fixture::new(&[("450814997", "CBA_A3", "CBA.pbo")], LISTING_SYMLINK);

    fixture
        .command()
        .arg("server")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("symlink"));
}

#[test]
fn an_unsigned_pbo_warns_by_default_and_fails_when_required() {
    let mods = [("450814997", "CBA_A3", "CBA.pbo")];

    let fixture = Fixture::new(&mods, LISTING_UNSIGNED);
    fixture
        .command()
        .arg("server")
        .assert()
        .success()
        .stdout(predicate::str::contains("[NO SIGNATURE] CBA.pbo"));

    let fixture = Fixture::new(&mods, LISTING_UNSIGNED);
    fixture
        .command()
        .args(["server", "--require-signatures"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("server problem(s) found"));
}
