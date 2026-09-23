# Contributing

Thanks for helping with the UKSFTA Tools. This guide covers how to build,
test, and submit changes.

## Prerequisites

- Rust (stable toolchain) — install with [rustup](https://rustup.rs/)
- `git-cliff` for release notes (optional, used at release time)
- `shellcheck` and `pwsh` with PSScriptAnalyzer for the install scripts

## Build and test

```bash
# Build the debug binary
cargo build

# Run all tests
cargo test

# Run the linters CI enforces
cargo fmt --check
cargo clippy -- -D warnings
```

## Project layout

- `src/main.rs` — CLI definition, dispatch, and module wiring
- `src/investigate/` — PBO origin tracing, Workshop search, scoring
- `src/sync/` — cache sync, audit, verify, identify, updates, size
- `src/modlist/` — `mod_sources` parsing, migration, import, HTML modlists
- `src/error.rs`, `src/atomic.rs`, `src/version.rs`, `src/prefix.rs` —
  shared error, atomic-write, version-check, and prefix helpers
- `src/steam.rs`, `src/util.rs`, `src/pbo.rs`, `src/lock.rs`, `src/origin.rs`,
  `src/workshop_api.rs` — supporting modules
- `mod_sources.txt` — mod list in TOML v2 format (see README)
- `install.sh`, `install.ps1` — platform installers
- `.cliff.toml` — git-cliff changelog configuration

## Making changes

1. Create a branch from `main`:
   `git checkout -b feat/<issue-id>-<description>`
2. Make your change. Add a unit test for any new logic.
3. Run the CI gate locally: `cargo fmt --check`, `cargo clippy -- -D
   warnings`, `cargo test`.
4. Release notes are generated at tag time by `git-cliff`. CI does not
   check the changelog, and there is no need to edit `CHANGELOG.md` by hand.
5. Commit with a conventional message (see below) and a GPG signature.
6. Push and open a pull request against `main`.

## Commit messages

Use [conventional commits](https://www.conventionalcommits.org/):

- `feat:` for a new feature
- `fix:` for a bug fix
- `docs:` for documentation
- `ci:` for CI changes
- `chore(deps):` for dependency bumps

Reference the issue or pull request number in the message where it
applies. Sign every commit with GPG.

## Pull requests

The `main` branch is protected. Changes go through a pull request. Every
PR runs the full CI gate (fmt, clippy, tests, script lint, security
scans). A PR merges only when all checks pass.

## Release process

Releases are cut by pushing a `v*` tag. The release workflow:

1. Runs the test gate (`cargo fmt --check`, `cargo clippy`, `cargo test`).
2. Builds the Linux and Windows binaries.
3. Generates release notes with `git-cliff` for this tag.
4. Creates `SHA256SUMS`, signs the binaries with sigstore, and creates the
   GitHub release.

There is no manual release step beyond the tag.
