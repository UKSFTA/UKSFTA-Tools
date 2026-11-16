# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project uses [Semantic Versioning](https://semver.org/).

## [0.10.0] - 2026-11-16

### Added

- A learned evidence pool. Every authored-text entry in a PBO is read,
  title-shaped strings are ranked by how rare they are in the local pack,
  and `$STR_*` references are resolved through CSV and XML stringtables.
  Binarised `\0raP` configs are parsed directly, which recovers
  `CfgPatches`, `requiredAddons`, `CfgMods` and author facts for PBOs that
  ship no readable `config.cpp`.
- Cross-query agreement. When two distinct queries both place the same
  Workshop item in their top five, and no other item agrees, the match is
  confirmed even if the per-query margin was a tie, provided the agreed
  item also has a strong title match or something in the PBO corroborates
  it. Two queries agreeing on a five-way tie is not enough.
- The category from a `CBA_fnc_addKeybind` registration is now an identity
  source of its own, ranked just below `CfgMods.name`. Modders usually put
  the mod title or the Workshop name in that category, which is what makes
  `ADE` resolve to `Advanced Diving Environment` instead of an item name.
- Strict acronym matching. A query token of three to six letters may match
  a contiguous run of title words whose initials spell it (`ADE` against
  `Advanced Diving Environment`), with no credit for scattered initials.
- Item-page author corroboration for the narrow case where every title
  gate already passes but nothing corroborates the file. It can never
  rescue a candidate that fails a title gate, and one read serves every
  PBO that points at the item.
- When several candidates tie on the same exact title, as five ADE variants
  do, the line reports the words they share as a shared title and prints no
  id. The name is useful; the id would be a guess.
- A tie is resolved by content, not by name. When candidates share an exact
  title, their item pages are checked for the PBO's stem, prefix or
  `CfgPatches` class, then for the uploader name. Exactly one match
  confirms the id. Zero or several keep the shared title.
- An unconfirmed cluster with a specific query may fetch page 2 of the
  Workshop search. Rare-word rarity is learned from every installed
  Workshop PBO, not only the current pack.
- Live progress during a scan. One line is rewritten in place with the
  completed queries, the percentage, the elapsed time, the estimated time
  remaining and the PBO in hand, so a full pack reports continuously
  instead of going quiet for minutes.
- The store records the top ten candidates per PBO, and
  `tools/calibrate.py` measures precision at rank 1, 3 and 10 against
  `.uksfta/labels.json`. Blank labels are skipped.

### Changed

- `investigate --online` now resolves each PBO's identity from the PBO's
  own config and prints a Workshop **title only for a confirmed match**.
  It reads `CfgMods`, a bracketed author tag, `CfgPatches`, display names,
  and title-shaped strings that are rare in the pack, then runs at most
  three paced Workshop searches per PBO. The unit of record is the PBO.
  Every other PBO prints one of `[ambiguous]`, `[no-query]`, `[no-result]`,
  `[rate-limited]`, or `[unresolved]` with a score and no name, and the
  summary counts each status. A scan has no request ceiling. It paces
  every query to the endpoint budget, waits out a rate limit with an
  exponential backoff of up to five retries, and continues until the pack
  is done, so a 203-PBO pack is covered in one run.
- `investigate --report` now renders from `.uksfta/identities.json`. An
  online run writes one record per PBO: its fingerprint, its evidence, the
  queries it tried, and the decision. `--report` reads that file and prints
  the same lines with no network, so a run can be inspected later. A PBO
  absent from the store, or present under a different fingerprint because
  the file changed, prints `[unresolved]` with no query. The store never
  holds a title for a non-confirmed PBO. The file is gitignored, and a
  schema or score-version bump discards it rather than converting records
  that came from an older ranking.
- The local origin block (`[UNKNOWN]`, `[RESOLVED]`, `[PACK]` lines) is
  unchanged and still prints before the resolution lines.

### Fixed

- The anchor is now a hard gate on every confirmation path. A title that
  missed the PBO's anchor token is capped, and the cap could pass the
  evidence gate at exactly its own value, so an unanchored title could be
  confirmed. Agreement between two queries and page-two results now also
  require a matched anchor and a specific query.
- A shared title is now pruned from a page-two decision as well as a
  page-one decision, so a name prints only when the PBO's own words
  anchor it.
- A `503` is no longer treated as a rate limit. It gets two short retries
  and the status, so one transient server fault cannot hold a scan for
  eleven minutes. A `429` that carries `Retry-After` is honoured, capped
  at five minutes.

### Removed

- The v1 identity derivation: prefix and `$STR_` search terms, short author
  handles, `requiredAddons` roots, the grouped online search, the keyed
  `QueryFiles` path, the `changelog` cross-reference scrape, and the v1
  `.uksfta/identities.json` cache. The v1 cache keyed on search terms, so
  it is discarded on sight rather than converted.
- The browse-page link scrape and the `filedetails/?id=` fallback. A page
  without the current SSR payload is a typed error, never a list of ids.

## [0.8.0] - 2026-09-24

### Added

- Dependency resolution for every enabled mod with `sync --resolve-deps`.
  A cached dependency is repacked into `addons/` and recorded in
  `mods.lock`. Discovered ids are persisted to the parent mod's
  `dependencies` array in `mod_sources.txt`.
- `sync --fill-from <MODLIST.html>`: satisfy discovered dependencies from
  an Arma 3 launcher modlist. A dependency the modlist provides is
  tracked as ignore and is not repacked. Workshop collections contribute
  their member ids from the Steam API.
- The non-mod Steam app ids `107410` and `228800` are filtered from
  required items.

### Changed

- A plain `sync` replays the persisted dependencies with no network.

### Fixed

- Required items of a fully cached mod, such as RHS Plus, were neither
  reported nor repacked. A required dependency absent from the cache now
  fails the run before any PBO is copied.
- `audit` now includes declared dependencies and no longer reports their
  PBOs as orphans.

## [0.7.0] - 2026-09-24

### Added

- `size` command: list every mod's download size from an Arma 3 launcher
  preset (HTML) and the total. Add `--online` to fetch sizes for mods
  that are not installed locally from the keyless Workshop API. On a
  terminal, sizes are coloured by their share of the total.
- Typed errors that map to documented exit codes: 0 for success, 1 for a
  failed check, 2 for missing or unparseable input. This lets `verify`
  and `audit` gate a build in CI.

### Changed

- `mods.lock` and `mod_sources.txt` are written atomically with a `.bak`
  backup. A corrupt lock is a hard error and is never replaced by an
  empty one.
- The legacy `[ignore]` marker is matched exactly, so a mod comment that
  mentions it no longer drops the mod. Migration to TOML v2 runs only
  from `sync` and `import`, never from a read-only command.
- `sync --offline` skips dependency resolution instead of being ignored.
- Online `investigate` caches results only on success, so a network
  failure is retried on the next run.
- Scraped and API text is truncated on character boundaries and stripped
  of terminal control characters before printing.
- The Workshop API calls share one client, one request builder and one
  100-id batching rule.

### Fixed

- Modlist import into a CRLF `mod_sources.txt` no longer corrupts the
  file.
- `extract_id` no longer mistakes `steamid=` or `valid=` for a Workshop
  id, and `toml_escape` escapes DEL.
- PBO config keys are matched exactly, so `namespace` and `authorName`
  no longer shadow `name` and `author`.
- A single unreadable cache, candidate or subdirectory no longer aborts
  a whole command.
- `file_sha256` updated for the sha2 0.11 digest API, which repairs the
  build after the sha2 dependency bump.

### Security

- Release blobs are signed with keyless sigstore over GitHub OIDC. The
  installers verify the bundle with cosign when it is available.
- CI builds with `--locked`, runs `cargo deny`, pins scanner versions
  and applies least-privilege token permissions.
- Dependency upgrades: rustls 0.23.45 for a TLS advisory, and
  indicatif 0.18 which removes the unmaintained `number_prefix` crate.

### Miscellaneous

- `investigate`, `sync` and `modlist` are split into focused modules.
  `main.rs` now holds only the CLI. Shared `workshop_api`, `atomic`,
  `prefix` and `version` modules remove duplicated logic.
- Added `rust-toolchain.toml`, `deny.toml` and package metadata.

## [0.6.0] - 2026-09-09

### Added

- `investigate` groups PBOs by mod family and searches the Workshop once
  per family instead of once per PBO. Each family search tries multiple
  variations: the full name, word parts, prefix-path segments, the
  CfgPatches class name, the author handle, and required-addon roots.
- New scoring signals: word-level title overlap, CfgPatches class name,
  author handle, description content, prefix path, dependency graph and
  PBO-name words. On the UKSF mod pack 34 of 92 groups resolve to a
  confident match with no false positives.
- Short search terms (under 5 characters) must match a title as a whole
  word or a suffix. This rejects prefix noise ("sty" in "style") while
  keeping compound acronyms ("nvg" in "GPNVG18").
- Colour-coded report output: green for a confident match, yellow for
  weak, red for unresolved. Colour turns off automatically when stdout
  is piped.
- Progress counter and final summary block in the report.

### Changed

- The identity cache now stores the full candidate data, so offline
  scoring matches online scoring. Previously a cached run zeroed
  popularity and quality signals and could flip a confident match to
  weak.

### Fixed

- Cache writes are atomic (temp file then rename). An interrupted run
  no longer corrupts the cache and forces a full re-search.

## [0.5.0] - 2026-09-08

### Added

- `investigate --online` uses Steam's official `QueryFiles` API when the
  `STEAM_API_KEY` environment variable is set. The keyed search ranks
  results properly and surfaces mods the browse-page scrape buries (for
  example `Zulu Custom`). Falls back to the keyless scrape without a
  key. The key is read from the environment only, never embedded in the
  binary.

### Changed

- Split the single `src/main.rs` (3480 lines) into focused modules:
  `util`, `steam`, `pbo`, `lock`, `modlist`, `origin`, `investigate`,
  `sync`. Pure structural refactor — no behaviour change.

## [0.4.1] - 2026-09-08

### Fixed

- `install.sh` no longer fails with `TMPDIR: unbound variable` on systems
  without `TMPDIR` set; it now defaults to `/tmp`.

## [0.4.0] - 2026-09-08

### Added

- Search the Workshop online for PBO origins that are unknown or
  pack-only. Search terms are derived from the PBO content in order of
  reliability: a non-vanilla `requiredAddons[]` root from the plain-text
  config, a mod-family string-table token, a short author handle, then
  the header prefix.
- Cache Workshop search results in `.uksfta/identities.json`
  (gitignored, never pushed) so repeat investigations are instant and
  fully offline.
- Cache confirmed (id, title) pairs, not just candidate IDs, so cached
  runs need no network.

### Changed

- `investigate` origin matching now uses the PBO header prefix (which
  survives re-packing) instead of byte hashing, so repacked pack copies
  resolve to their original mod.
- Origin arbitration prefers standalone mods (fewest distinct prefix
  roots) over aggregate packs; a folder being investigated is excluded
  from its own candidates.
- PBO folder statistics are precomputed once; lazy hashing only runs
  when the prefix path fails. The 198-PBO pack scan dropped from 62s to
  0.015s.
- `sync` copy loop shows a progress bar.

## [0.3.0] - 2026-09-08

### Added

- Add version command and update check (#48)

- Add investigate command to trace untracked PBO origins (#47)

- Add install.sh and script linting to CI (#46)

- Add install.ps1 and SHA256SUMS for easier installation (#44)

- Accept Workshop URLs in TOML mod_sources id field (#33)

- Upgrade mod_sources.txt to TOML format with metadata (#32)

- Add import command for Arma 3 launcher modlists (#31)

- Add --resolve-deps flag to resolve Workshop dependencies (#30)

- Add --modlist flag for Arma 3 launcher preset generation (#29)


### CI

- Group release changelog by conventional commit type

- Fix release artifact glob to exclude .d files


### Dependencies

- Bump actions/download-artifact from 4 to 8 (#42)

- Bump actions/upload-artifact from 4 to 7 (#41)

- Bump actions/cache from 4 to 6 (#40)

- Bump toml from 0.8.23 to 1.1.5+spec-1.1.0 (#37)

- Bump scraper from 0.22.0 to 0.27.0 (#38)

- Bump reqwest from 0.12.28 to 0.13.4 (#39)


### Documentation

- Add auto-generated changelog and repo hygiene files (#49)


### Fixed

- Make changelog check immune to squash-merge PR suffix (#51)

- Sync changelog after squash merge and restore CI check (#50)

- Escape untrusted mod names in HTML and TOML output (#36)

- Remove emojis from issue and PR templates (#35)

- Wire mod metadata from TOML sources into mods.lock (#34)


### Miscellaneous

- Bump version to 0.3.0 for release (#52)

- Bump version to 0.2.0 for release (#43)

<!-- generated by git-cliff -->
