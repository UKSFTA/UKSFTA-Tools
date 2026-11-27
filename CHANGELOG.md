# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
This project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- `--password-prompt` asks once, before the probe and the transfer. The
  progress view is hidden while the prompt runs. The system probe never
  prompts, and a probe auth failure that uses the typed password is fatal.
- The system probe models the `--jobs` path with up to four parallel 1 MiB
  uploads and reports the aggregate rate and job count. `segment_probe`
  gains `system_jobs`.
- A link probe chooses the whole-file path when it is faster. It runs for a
  segmented file of 256 MiB or more, moves 4 MiB four times, and prints the
  estimates and the decision. `--segment-probe` is on by default and
  `--no-segment-probe` turns it off.
- `--segment-fsync-every N` fsyncs every fourth completed range by default.
  A resume re-hashes the prefix, so a checkpoint may safely run ahead of the
  last fsync.
- The read-back hash uses a dedicated 261120-byte buffer.
- The segmented published line reports `max`, `start` and `peak`
  connection counts, and each adaptive change logs one line.
- `--limit` caps the upload rate on both paths. A `K`, `M` or `G` suffix is
  1024-based. The segmented path uses one shared limiter for every
  connection, and the system `sftp` path passes `-l` in Kbit/s.
- `--preallocate` sets the segmented temp file to its full length before
  the first range and is on by default. `--no-preallocate` turns it off. A
  server without the request gives one warning and the upload continues.
- `--notify` sends one desktop notification when the run finishes. The body
  names the pack, the result, the file count, the bytes and the elapsed
  seconds.
- `--segment-adaptive` scales the segmented connections between one and
  `--segment-connections`, which is now the maximum. The run starts at four,
  changes one connection per sample window, and honours the server's session
  limit.
- An interrupted segmented run prints
  `segment: resuming at <percent>% (<bytes> of <total> bytes)` and starts
  the total bar at the acknowledged bytes.
- A summary table after an upload shows the file, bytes, seconds, rate,
  SHA-256 and remote path, with a total row. Whole-file seconds and rate are
  marked `~` because they are a byte-share estimate. The same values reach
  the JSON `uploads[]` entries.
- One whole-upload progress view: a total bar, one row per planned file
  with `waiting`, `uploading` and `done`, and the segmented file's
  connection rows beneath its own row. A redirect gets one plain line
  every five seconds, and `--progress-json` writes one JSON object per
  line on standard error. A segmented fallback now prints
  `segment: not used: <kind>: <message>; using the whole-file path`.

- `uksfta server --json` writes one machine-readable object to standard
  output, with the plan, counts, timings and a result. `--doctor` is a
  read-only preflight of the connection, the pack folder and free space.
  Every human run prints one `timing:` line on standard error.

- `--segmented` sends one large upload over several connections, verifies
  the temporary file by SHA-256 and publishes it with an atomic rename.
  It is off by default and needs key or agent authentication.

- A segmented upload resumes from a checkpoint of the acknowledged
  ranges. The checkpoint holds the local size, modification time and
  SHA-256. The remote prefix is hashed and compared before it is
  continued, and the remote size is never trusted. `--segment-resume
  auto|off` selects the behaviour.

- `--segment-reconnects` replaces a dead segmented connection and resends
  only its in-flight range, with waits of 1 s, 2 s, 4 s and 8 s.

- Stale segmented temporary files more than a day old, and temporary
  files with no checkpoint, are removed with a warning.

- A shaped-link test harness compares a segmented upload with one stream,
  and covers a killed session, a loss burst and three interrupted resumes.
  It runs only with `UKSFTA_SFTP_SHAPED=1` on a kernel with `sch_netem`,
  and skips with a printed reason otherwise.

- The README documents every segmented flag, the read-back hash, the
  acknowledged prefix and the exit codes. CI builds and tests on Windows,
  and runs the suite with every feature.

- `server --password-prompt` reads one password from the controlling
  terminal for the segmented path. It takes no value, has no config key
  or environment variable, makes one attempt and stores nothing. Without
  a terminal it exits 2 and names the other methods. The bytes are not
  wiped, and that limit is documented.

- `--legacy-sftp` forces the system `sftp` path for one release and
  conflicts with `--segmented`. A config `segment = true` now takes effect
  without the flag. The JSON report adds `server.segment` and `uploads`,
  and the doctor reports the segment settings and the server extensions.

  The default transport stays the system `sftp` path. The flip is a
  separate change. It keeps `--legacy-sftp` for one release and removes
  the flag after that. The flip needs these five results:

  (a) 20 consecutive live uploads of the 977 MB PBO with equal SHA-256;
  (b) the Docker suite green on OpenSSH 9.6 and 10.6;
  (c) 10 forced-kill resumes with equal hashes and no temp residue;
  (d) host key and agent auth confirmed on the real panel;
  (e) one release cycle with no defect report.

- Opt-in launch-line and `server.cfg` checks. `--launch-line` reads a
  local copy and finds `@<id>` in `-mod` or `-serverMod`. `--server-cfg`
  reads one file over SFTP and checks `verifySignatures` and
  `allowedFilePatching`. The lines warn by default, and `--strict-config`
  makes a failing check exit 1.

- `uksfta deps` reports every `CfgPatches` class the pack provides and
  every `requiredAddons` entry it needs. Each requirement is provided,
  external (listed once in `.uksfta/external-addons.txt`) or unaccounted.
  The check is advisory, `--strict` exits 1 on an unaccounted entry, and
  `--suggest` prints the external-list format. `--mission` adds a
  mission's `addons[]` list.
- Compare and upload the whole release root at any depth, not only
  `addons/` and `keys/`. Files that start with `.`, `*.biprivatekey`,
  `*.tmp`, `*.bak` and symbolic links are excluded. `meta.cpp` and
  `mod.cpp` keep their own checks, and prune still removes only
  `addons/*.pbo`.
- Match each local `.bisign` authority against the embedded names in
  `keys/*.bikey`, locally and on the server. A renamed key still matches,
  because the file names are ignored. The command does not verify
  cryptography, a hash or a PBO.

### Changed

- The minimum supported Rust version is 1.89, for the native transport
  dependencies.

### Security

- Pin the sftp host key on every run: `StrictHostKeyChecking=yes` by
  default, `UpdateHostKeys=no`, and an explicit key file from
  `--known-hosts` or `known_hosts`. A failure names the destination, the
  key file and the remedy. `--host-key-policy accept-new` prints its own
  acceptance notice because the error log level hides the client's.

## [0.10.1] - 2026-11-26

### Fixed

- Refresh the dependency tree to clear the RUSTSEC advisories in `url`,
  `quinn-proto`, `quinn-udp` and `tokio`. `cargo audit` and `cargo deny
  check` pass.

## [0.10.0] - 2026-11-26

### Added

- `uksfta size` with no argument reports the installed pack size: each
  mod's PBOs from `mods.lock` measured in `addons/`, the share of the
  tracked total, untracked PBOs and missing locked PBOs. `--pbo` lists
  each PBO, and `--sort name` orders the report by name.
- `uksfta export` writes the enabled entries from `mod_sources.txt` as an
  Arma 3 launcher preset. The preset name defaults to the current
  directory name and can be set with `--preset-name`. `--dry-run`
  previews the rows.
- A learned evidence pool. Every authored-text entry in a PBO is read,
  title-shaped strings are ranked by how rare they are in the local pack,
  and `$STR_*` references are resolved through CSV and XML stringtables.
  Binarised `\0raP` configs are parsed directly, which recovers
  `CfgPatches`, `requiredAddons`, `CfgMods` and author facts for PBOs that
  ship no readable `config.cpp`.
- Cross-query agreement. When two distinct queries both place the same
  Workshop item in their top five, and no other item agrees, the match is
  confirmed even if the per-query margin was a tie, but only when a link
  ties the item to the file: a Workshop URL, a dependency root, the
  uploader name, or the item page, and it must lead every rival in the pool
  by the margin a single query needs. Agreement on a generic title alone is
  never enough, a shared word such as `markers` is not a link, and a family
  of identically titled variants stays ambiguous.

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

### Fixed

- A scan no longer records `no-result` when Steam answers a search with an
  empty page while throttling. Page one is fetched a second time after a
  pause, and only then is an absence recorded. On one 203-PBO run thirteen
  files were wrongly recorded as absent in a single seventy-second window.
- Content corroboration no longer confirms a weak title. It could rescue a
  title at 0.57 that shared an object name with another mod's item, and
  print that mod's name. It may now only rescue a missing head noun or a
  thin margin.
- Agreement can no longer pick a family member. A 203-PBO run printed
  `[A.D.E.] Advanced Diving Environment - Reloaded by Brigata Lancieri` for
  `ade_core.pbo` while the human label names `ADE - Advanced Diving
  Environment`; six variants tie at a perfect score, so the line now reports
  the shared title with no id.
- Cross-query agreement no longer confirms on a shared word. Two broad
  queries could agree on one of ten titles containing `markers` or
  `backpacks` and print it. Agreement now needs a link to the file, so a
  private pack that ships its own backpacks is reported ambiguous instead
  of being attributed to an unrelated public mod.
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

### Security

- Two certificate-validation advisories are cleared by updating
  `rustls-webpki` to 0.103.17 and `webpki-root-certs` to 1.0.12.

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
