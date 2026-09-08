# Contributing

Thanks for helping with the UKSFTA Tools. This guide covers how to build,
test, and submit changes.

## Prerequisites

- Rust (stable toolchain) — install with [rustup](https://rustup.rs/)
- `git-cliff` for the changelog (optional, CI checks it)
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

- `src/main.rs` — the single-file CLI (parsing, sync, audit, import,
  investigate, version)
- `mod_sources.txt` — mod list in TOML v2 format (see README)
- `install.sh`, `install.ps1` — platform installers
- `.cliff.toml` — git-cliff changelog configuration

## Making changes

1. Create a branch from `main`:
   `git checkout -b feat/<issue-id>-<description>`
2. Make your change. Add a unit test for any new logic.
3. Run the CI gate locally: `cargo fmt --check`, `cargo clippy -- -D
   warnings`, `cargo test`.
4. If you change the changelog, regenerate it:
   `git-cliff v0.1.0..HEAD -o CHANGELOG.md`
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
PR runs the full CI gate (fmt, clippy, tests, changelog check, script
lint, security scans). A PR merges only when all checks pass.

## Release process

Releases are cut by pushing a `v*` tag. The release workflow:

1. Builds the Linux and Windows binaries.
2. Generates release notes with `git-cliff`.
3. Creates `SHA256SUMS` and the GitHub release.
4. Updates the committed `CHANGELOG.md` and pushes it back.

There is no manual release step beyond the tag.
