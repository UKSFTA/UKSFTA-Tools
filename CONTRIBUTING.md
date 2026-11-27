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
- `src/docs/` — `docs` command: project discovery, version, description, changelog, name lookup, composition state
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

   The branch starts with one of seven types: `feat`, `fix`, `docs`,
   `refactor`, `test`, `ops` or `sec`. Add the issue id when there is one.
   Without an issue, use `<type>/<description>`. A performance change takes
   `feat` when it changes behaviour and `refactor` when it does not.
2. Make your change. Add a unit test for any new logic.
3. Run the CI gate locally: `cargo fmt --check`, `cargo clippy -- -D
   warnings`, `cargo test`.
4. Release notes are generated at tag time by `git-cliff`. CI does not
   check the changelog, and there is no need to edit `CHANGELOG.md` by hand.
5. Commit with a conventional message (see below) and a GPG signature.
6. Push and open a pull request against `main`.

## Commit messages

Use [conventional commits](https://www.conventionalcommits.org/), with the
seven types above:

- `feat:` for a new feature or a behaviour change
- `fix:` for a defect fix
- `docs:` for documentation
- `refactor:` for a change that keeps behaviour
- `test:` for tests only
- `ops:` for build, CI and dependency changes
- `sec:` for a security fix

Reference the issue or pull request number in the message where it
applies. Sign every commit with GPG.

## Performance checks

Run these before a change that touches a hot path: the upload, the verify
pass, PBO parsing, hashing, or a filesystem walk.

| Check | Command | Finds |
| --- | --- | --- |
| Pattern scan | `npx -p @ast-grep/cli sg scan src` | A client or a process built inside a loop, and an uncapped whole-file read |
| Benchmarks | `cargo bench` | Time and allocation count per call, through divan |
| Allocation profile | `heaptrack target/release/uksfta server ...` | The call site that allocates |
| Async tasks | `RUSTFLAGS="--cfg tokio_unstable" cargo run --features console -- server ...` then `tokio-console` | A task that waits on the peer while others sit idle |
| Binary size | `cargo bloat --release --crates` | The crates that dominate the binary |
| Monomorphisation | `cargo llvm-lines --release` | Generic code that is copied per type |
| Unused crates | `cargo machete` | A dependency to remove |

The scan runs in CI. A rule in `.ast-grep/rules` fails the build when it
matches at warning severity or above. Add a rule when a cost is specific to
this codebase and a linter cannot see it.

`cargo bench` runs `benches/hot_paths.rs` against a 64 MiB hash sample and a
4 MiB config-like buffer. Divan reports an allocation count, so a buffer built
per iteration shows up as a change in that column.

Gungraun, the maintained form of iai-callgrind, counts instructions
deterministically and suits a CI regression on a pure function. It needs
Valgrind, so it is not part of this pipeline.

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
