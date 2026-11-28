# UKSFTA-Tools

Shared tooling for the UKSF Task Force Alpha Arma 3 workspace, written in
Rust.

[![Rust](https://github.com/UKSFTA/UKSFTA-Tools/actions/workflows/rust.yml/badge.svg)](https://github.com/UKSFTA/UKSFTA-Tools/actions/workflows/rust.yml)
[![Release](https://img.shields.io/github/v/release/UKSFTA/UKSFTA-Tools)](https://github.com/UKSFTA/UKSFTA-Tools/releases)
[![Licence](https://img.shields.io/badge/licence-MIT-blue)](LICENSE)
[![Rust version](https://img.shields.io/badge/rust-1.89%2B-orange)](Cargo.toml)

Sync a modpack from the Steam Workshop, audit every PBO, check the game
server over SFTP, and upload only the files that changed.

Install with [`install.ps1`](install.ps1) on Windows or
[`install.sh`](install.sh) on Linux and macOS. See
[Installation](#installation).

## Contents

- [Getting started](#getting-started)
  - [Installation](#installation)
  - [Usage](#usage)

- [Guides](#guides)
  - [Publish files](#publish-files)
  - [Missing mods](#missing-mods)
  - [Importing a modlist](#importing-a-modlist)
  - [Exporting a modlist](#exporting-a-modlist)
  - [Modlist size](#modlist-size)
  - [Investigating PBO origins](#investigating-pbo-origins)
  - [Server verification over SFTP](#server-verification-over-sftp)
  - [Dependencies](#dependencies)
  - [Update checking](#update-checking)

- [Reference](#reference)
  - [Input format](#input-format)
  - [Output](#output)
  - [Exit codes](#exit-codes)
  - [Platforms](#platforms)

- [Licence](#licence)
- [Maintainers](#maintainers)

## Getting started

### Installation

#### Windows (recommended)

Run this in PowerShell:

```powershell
irm https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.ps1 | iex
```

This downloads `uksfta.exe`, verifies its SHA256 checksum against the
release's `SHA256SUMS`, installs it under `%LOCALAPPDATA%\Programs\uksfta`,
and adds it to your user PATH. Open a new terminal and run `uksfta --help`
to verify.

#### Linux / macOS

Run this in a terminal:

```sh
curl -fsSL https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.sh | sh
```

This downloads the matching binary, verifies its SHA256 checksum against the
release's `SHA256SUMS`, and installs it to `~/.local/bin` (or `/usr/local/bin` for
a root install on macOS). The installer selects the asset for Linux x86_64, Linux
arm64 or macOS arm64. Open a new terminal and run `uksfta --help` to verify.

#### Manual download

<details>
<summary>Assets, checksums and a source build</summary>

Download the binary for your platform from the [Releases
page](https://github.com/UKSFTA/UKSFTA-Tools/releases) and put it on your
`PATH`:

| Platform | Asset |
| --- | --- |
| Linux x86_64 | `uksfta` |
| Linux arm64 | `uksfta-linux-arm64` |
| macOS arm64 | `uksfta-macos-arm64` |
| Windows x86_64 | `uksfta.exe` |

```bash
# Linux and macOS: rename the asset to uksfta when it has a suffix
chmod +x uksfta-linux-arm64
sudo mv uksfta-linux-arm64 /usr/local/bin/uksfta

# Windows: put uksfta.exe on PATH
```

Each release also includes a `SHA256SUMS` file so you can verify the
binary you downloaded.

Or build from source: `cargo build --release` (Linux, macOS) /
`cargo build --release --target x86_64-pc-windows-msvc` (Windows, matches
the published release binary). The minimum supported Rust version is 1.89.

</details>

### Usage

Run from a mod repository root (a directory containing `mod_sources.txt` and `addons/`).

```bash
# Copy PBOs from the local Steam Workshop cache into addons/, write mods.lock
uksfta sync

# Confirm all locked PBOs are present
uksfta verify

# Verify the game server's workshop tree over SFTP
uksfta server

# Upload missing PBOs and signatures
uksfta server --upload
```

<details>
<summary>Every command and example</summary>

```bash
# Preview what a sync would change
uksfta sync --dry-run

# Generate an Arma 3 launcher modlist for missing mods
uksfta sync --modlist

# Custom output path for the modlist
uksfta sync --modlist --modlist-path ./my-modlist.html

# Also resolve dependencies from Steam Workshop pages
uksfta sync --modlist --resolve-deps

# Skip dependency resolution and stay offline
uksfta sync --offline

# Show which PBOs came from which Workshop mod
uksfta identify

# Audit each mod's PBOs against addons/ (present/missing per PBO)
uksfta audit

# Show only missing PBOs and per-mod counts
uksfta audit --missing-only

# Compare mods.lock timestamps against the Workshop cache
uksfta updates

# Trace untracked PBOs in addons/ back to their Workshop origin
uksfta investigate

# Resolve each PBO's identity online; a title prints only when confirmed
uksfta investigate --online

# Investigate all PBOs, including tracked ones
uksfta investigate --all

# Refresh the online resolution, then print the status lines
uksfta investigate --report --online

# Print the stored resolution lines with no network
uksfta investigate --report

# Upload the missing files, then remove extra PBOs
uksfta server --upload --prune

# Check every required addon is provided by the pack or the external list
uksfta deps

# Include a mission's addons[] list, and fail on an unaccounted addon
uksfta deps --mission mission.pbo --strict

# Show the installed version and force a fresh update check
uksfta version

# Update the HEMTT publish description and changelog
uksfta docs

# Preview the generated text without changing files
uksfta docs --dry-run

# Update every HEMTT project below a directory
uksfta docs --root ../

# Fail when a configured publish file is missing or stale
uksfta docs --check

# Look up missing mod names from the Steam Workshop
uksfta docs --online

# Refetch cached names after a Workshop rename
uksfta docs --online --refresh-names

# Replace a hand-written description or changelog section
uksfta docs --force

# Import mods from an Arma 3 launcher modlist HTML file
uksfta import ./my-modlist.html

# Preview what would be imported without changing mod_sources.txt
uksfta import ./my-modlist.html --dry-run

# Export mod_sources.txt as an Arma 3 launcher modlist
uksfta export

# Export to a chosen path with a chosen preset name
uksfta export ./modlist.html --preset-name "UKSFTA Modpack"

# Print the entries without writing a file
uksfta export --dry-run

# Show each mod's size from a launcher modlist HTML, and the total
uksfta size ./my-modlist.html

# Also fetch sizes for mods not installed locally (Workshop API)
uksfta size ./my-modlist.html --online

# Show the installed pack size per mod from mods.lock and addons/
uksfta size

# Print one line per PBO, and sort the report by name
uksfta size --pbo --sort name
```

</details>

## Guides

### Publish files

`uksfta docs` reads `.hemtt/project.toml` and acts only when the project
sets a publish path:

```toml
[hemtt.publish]
description = "workshop_description.md"
changelog = "CHANGELOG.md"
```

The command writes the description from the project name, the version
file, and `mods.lock`. It lists the installed mods, then the dependencies
that the content does not already include. HEMTT converts the Markdown to
Steam Workshop format at publish time.

The command uses the two paths exactly as written. They may have any name or
subdirectory. For example, AEE sets `description = ".hemtt/workshop.md"` and A3
Accessibility sets `description = "README.md"`.

With `--online`, the command asks the Steam Workshop API for the title of every
entry whose name is empty or the `Mod <id>` placeholder. It leaves real names
alone and makes one request for all missing ids.

The command stores the resolved titles in `.uksfta/workshop-names.json`, so
later offline runs keep them. Add `--refresh-names` with `--online` to refetch
cached titles after a Workshop rename.

The changelog follows Keep a Changelog 1.0.0. It records the mods added, updated,
and removed since the previous version, using `mods.lock` as the source. The
command caches only the previous composition, in `.uksfta/publish-state.json`.
Commit that file so a fresh clone and CI see the same baseline.

Every run rebuilds the section for the current version, so a repeated run
produces the same text. A version change starts a new section. Older sections
remain in the file, and the published change notes remain on Steam.

Generated descriptions end with `[//]: # (generated by UKSFTA-Tools)`, and every
generated changelog section contains the marker. It is a Markdown
link-reference definition, which HEMTT's converter drops, so it never reaches
Steam.

The command treats a file or section without the marker as hand-written. It
leaves the file alone, and `--check` reports it. Use `--force` to replace it.

HEMTT 1.22 looks up the changelog with the full version string, but its parser
accepts only `major.minor.patch`. The changelog therefore uses the three-part
form.

To let `hemtt publish` find the entry, set `[version] git_hash = 0` and remove
`BUILD` from `script_version.hpp`. The command warns when the project does not
meet this.

Run `docs` in a project root. Use `--root <PATH>` to search a directory
tree for `.hemtt/project.toml` files. `--dry-run` prints each target and
the generated text without writing files. `--check` writes nothing and
exits 1 when a target is missing or stale, so it can gate a build.

The command stays offline unless you pass `--online`. It reports and skips a
project without `[hemtt.publish]`, without a readable version, or without
`mods.lock`. It keeps the previous file as `.bak`.

#### CI

Run this step before `hemtt publish`:

```yaml
- run: uksfta docs --online --check
```

The step fails when a publish file is stale, so the release stops before
it uploads old text. To upload regenerated text, run `uksfta docs
--online` earlier in the job and commit the result, or run it in the
release job before `hemtt publish`.

### Missing mods

`sync` reports a mod it cannot find, and can open the Workshop page so
you can subscribe.

<details>
<summary>What sync reports, and how to subscribe</summary>

When a mod in `mod_sources.txt` is absent from every local Workshop cache, `sync`
reports it with its name and a direct link to the Steam Workshop page.

It then lists a set of `steam://url/CommunityFilePage/...` deep links, so you can
subscribe to each mod in Steam. Once you subscribe and download, re-run
`uksfta sync` to pull the PBOs into `addons/`.

Use `--modlist` to generate an Arma 3 launcher preset file (HTML) that
lists all missing mods. Open the file in the launcher to batch-subscribe
to every missing mod at once. The output defaults to `missing-mods.html`.
Use `--modlist-path` to choose another path.

Add `--resolve-deps` to fetch the Workshop page of every enabled mod. It
discovers dependencies not listed in `mod_sources.txt`. Resolution covers
every enabled mod, not only the missing ones.

The command repacks a discovered dependency that is already in the Workshop
cache into `addons/`, and records it in `mods.lock`. A later plain `sync` keeps
it. The command reports a dependency that is absent from the cache as missing,
and adds it to the modlist.

If a required dependency is absent from the cache, `sync` fails with exit code 1.
It fails before it copies any PBO, so a broken mod is never published. The
command ignores the non-mod Steam app ids `107410` and `228800`.

This requires a network connection and adds a one-second delay per mod for rate
limiting. A page that does not load is a warning for that mod, and the run
continues. When no Workshop page loads at all, `sync --resolve-deps` exits 2 and
resolves no dependencies.

</details>

### Importing a modlist

`import` reads a launcher preset file (HTML) and adds its mods.

<details>
<summary>Import rules and the dry run</summary>

Use `import` to add mods from an Arma 3 launcher preset file (HTML) to
`mod_sources.txt`:

```bash
# Add all Steam mods from a shared modlist
uksfta import ./my-modlist.html

# Preview first: prints what would be added, changes nothing
uksfta import ./my-modlist.html --dry-run
```

The command appends each Steam mod as `{id} # {name}`, and skips mods already
present in `mod_sources.txt`, including the `[ignore]` section. It skips a local
mod without a Workshop ID, with a warning. It inserts new entries before the
`[ignore]` section if one exists.

</details>

### Exporting a modlist

`export` writes every enabled mod as a launcher preset file (HTML).

<details>
<summary>Export details, naming and defaults</summary>

Use `export` to write every enabled mod in `mod_sources.txt` as an Arma 3
launcher preset file (HTML):

```bash
# Write modlist.html in the current directory
uksfta export

# Write to a chosen path
uksfta export ./out/modlist.html

# Set the preset name shown in the launcher
uksfta export --preset-name "UKSFTA Modpack"

# Print the entries and write no file
uksfta export --dry-run
```

The output path defaults to `modlist.html` in the current directory. The
preset name defaults to the name of the current directory, and falls back
to `UKSFTA Modpack` when that name is not usable. `--preset-name` sets the
name in the `arma:PresetName` meta tag and the page heading.

The command leaves out entries with `role = "ignore"` or `enabled = false`. The
mod order follows `mod_sources.txt`. It replaces a source name of `Mod <id>`
with the name in `mods.lock` when the lock holds one.

`--dry-run` prints the entries it would write, and writes no file.

The output is a normal launcher preset, so `uksfta import` reads it back.

The command cannot export a local mod. `mod_sources.txt` holds Workshop ids only,
and a local mod has no Workshop id.

</details>

### Modlist size

`size` totals the download size of a launcher preset, or of the
installed pack.

<details>
<summary>How sizes are read and coloured</summary>

Use `size` to list the download size of every mod in an Arma 3 launcher
preset file (HTML) and the total:

```bash
uksfta size ./my-modlist.html
```

Sizes come from the local Workshop cache (`appworkshop_107410.acf`), so the
command needs no network access. It lists a mod that is not installed locally as
`unknown` and leaves it out of the total. A local mod without a Workshop ID has
no size, and the command reports it separately.

Add `--online` to fetch sizes for mods not installed locally from the
keyless Steam Workshop API (`GetPublishedFileDetails`). One request covers
the whole list, up to 100 ids per call. Without the flag the command stays
offline.

When the output is a terminal, the command colours each size by its share of the
total: green under 5%, yellow under 20%, red at or above 20%. It shows a mod
with no known size in grey.

</details>

#### Installed pack size

<details>
<summary>The installed-size report and its flags</summary>

Run `size` with no file to report the installed size of each mod in
`mods.lock`. The command reads every PBO path in the lock and measures
the file in `addons/`:

```bash
uksfta size
```

The command sorts the report by size, largest first. Each line shows the size, the
share of the tracked total, the mod name, the Workshop ID, and the PBO count.

The command also reports PBOs that no lock entry tracks, and locked PBOs that are
absent from `addons/`. A missing PBO adds no bytes to the total.

Add `--pbo` to print each PBO and its size under its mod. Add `--sort name`
to sort the report by mod name in case-insensitive order. The default order
is by size:

```bash
uksfta size --pbo
uksfta size --sort name
```

With `--pbo`, the command also lists each untracked PBO with its size, and the
name of each locked PBO that is absent from `addons/`. Enabled mods in
`mod_sources.txt` that are not in `mods.lock` appear under `Not installed`, with
the name and the Workshop ID.

The command stays silent when there is no `mod_sources.txt`. It prints a warning
to stderr when the file exists and does not parse.

</details>

### Investigating PBO origins

Use `investigate` to trace PBOs in `addons/` back to their Workshop
origin. By default it examines PBOs not tracked in `mods.lock`, such
as leftovers from removed mods, merged packs, or manual copies:

<details>
<summary>Matching, online lookups and the result format</summary>

```bash
# Trace untracked PBOs to their source Workshop mod
uksfta investigate

# Resolve each PBO's identity online and confirm the name
uksfta investigate --online

# Investigate every PBO, including tracked ones
uksfta investigate --all

# Refresh the online resolution, then print the status lines
uksfta investigate --report --online
```

The tool matches each PBO by its **header prefix** (the addon's canonical path, e.g.
`z\ace\addons\grenades`) against the local Workshop cache. The winning mod's
`meta.cpp` then gives its Workshop ID and name. The prefix survives re-packing,
so a repacked copy still resolves to its original mod.

When a PBO exists in multiple cache folders, the tool prefers the standalone mod
over aggregate packs. It identifies a pack by its span of many distinct prefix
roots. It reports a PBO with no matching copy as unknown.

Such a PBO may come from a deleted or private mod, or a manual copy. The tool
flags a result whose best match is itself a pack as `[pack: verify source]`.

With `--online`, the tool also calls Steam's `GetPublishedFileDetails` API (no key
required) to report whether each origin still exists and is publicly visible. An
item that is not publicly visible is either removed or set private. The API
cannot tell the two apart without the owner authenticating.

The same run then resolves the identity of every examined PBO from its own content.
It reads the `CfgMods` name, a bracketed author tag, the `CfgPatches` class and
URL, the display names, and other title-shaped strings that are rare in the pack.

It plans at most three Workshop text searches per PBO, most specific first. It
stops at the first confirmed match.

A search result confirms only when the title covers the query phrase, the head noun
agrees, the result set is narrow, and the best candidate leads the next one by a
wide margin.

Content corroboration may rescue a missing head noun or a thin margin, but never
a title that matches the query weakly. A PBO whose config carries its own
Workshop URL confirms on that id alone.

Evidence comes from every authored-text entry, not only the config. Strings that
are rare in the local pack, stringtable values reached through `$STR_*`
references, and facts recovered from a binarised `\0raP` config all feed the
same gates.

A query token of three to six letters may match a contiguous run of title
initials, so `ADE` can match `Advanced Diving Environment`. The tool reads the
category of a `CBA_fnc_addKeybind` registration as a mod-name source of its own,
because modders usually put the title or the Workshop name there.

Two distinct queries that agree on the same item confirm it even when the per-query
margin is a tie, but only when a link ties that item to this file: a Workshop URL
in its config, a dependency root, the uploader name, or the item page itself.

The agreed item must also lead every rival in the result pool by the same margin
a single query would need. A family of six identically titled variants therefore
stays ambiguous.

Agreement on a generic title never confirms, and a shared word such as `markers`
or `backpacks` is not a link. Popularity never orders or confirms a candidate.

The tool resolves a tie by content where possible. It checks the tied candidates'
item pages for the PBO's stem, prefix or `CfgPatches` class, then for the
uploader name. Exactly one match confirms the id.

Otherwise the line keeps the shared title. Unconfirmed clusters with a specific
query may fetch page 2 of the Workshop search. The tool learns rarity from every
installed Workshop PBO rather than from the pack alone.

When several candidates tie on the same exact title, the line reports the
words they share as a shared title and prints no id, so a family name is
never turned into a guessed Workshop id.

The item page supplies the uploader's name. The tool uses it only as
corroboration, and only when every title gate already passes, because the
uploader is often a repacker rather than the original author. Each item costs
one read, however many PBOs point at it.

The store also records the top ten candidates for each PBO. After a run,
`tools/calibrate.py` compares those against `.uksfta/labels.json` and prints
precision at rank 1, 3 and 10 by status.

A run reuses a fresh record while the PBO fingerprint matches, and orders the rest
by oldest attempt. A scan has no request ceiling. It paces every query, waits out
a rate limit with an exponential backoff, and continues until it finishes the
pack.

The command rewrites one line in place while it runs. That line shows the
completed queries, the percentage, the elapsed time, the estimated time
remaining and the PBO in hand:

```text
  [ 41/163  25%] 03:12 elapsed  ETA 09:31  esp_objects
```

The output is one line per PBO. A confirmed match prints the title and the
Workshop id:

```text
[confirmed] ade_core.pbo -> "ADE - Advanced Diving Environment" (1891978580) [query "Diving Mask", M=1.00 Q=0.83 margin=1.00]
```

Every other PBO prints a status, a score, and no name. The status is one of
`[ambiguous]`, `[no-query]`, `[no-result]`, `[rate-limited]`, or `[unresolved]`.

A name appears only when the tool confirms it, so a doubtful candidate never
prints as a mod title. The summary counts each status.

The tool paces the browse endpoint at one request every four seconds, so a large
pack with many unseen PBOs takes minutes per run. A rate limit waits and retries
with a growing delay, up to five times, and only then records that PBO as rate
limited. The rest of the scan continues.

The tool writes every resolution to `.uksfta/identities.json`, so a later run
prints the same lines with no network. The command keys the file by PBO file
name, guards it with the PBO fingerprint, and git ignores it. A record carries a
title only when the tool confirmed the match.

```bash
# Print the stored resolution lines, offline
uksfta investigate --report

# Resolve first, then print (refreshes what the run reached)
uksfta investigate --report --online
```

`--report` reads `.uksfta/identities.json` and fingerprints every PBO in
`addons/`, so it reads the PBO file contents as well. A PBO that the file
does not describe, or describes under a different fingerprint because the
file changed, prints `[unresolved]` with no query. A missing file prints

```text
No cached resolutions. Run 'uksfta investigate --online' first.
```

and exits 0. The store records which ranking produced each decision. A schema or
score-version bump therefore discards the whole file instead of converting it.

The next `--online` run rebuilds the file from scratch. A `[rate-limited]` or
`[no-result]` record retries on the next online run.

The store also records the top ten candidates each PBO ranked, so you can examine
a mismatch after the run. `tools/calibrate.py` scores the store against
`.uksfta/labels.json`, which lists the Workshop id a person has verified for a
PBO. It reports precision at rank 1, 3 and 10 for each stored status, and exits
1 only when a confirmed row names the wrong id.

The `identify` command uses the same byte-hash matching, so its origin
reports are now deterministic too.

</details>

### Server verification over SFTP

`uksfta server` compares the private modpack in the local build output with the
pack folder on a game server. The pack id comes from `meta.cpp`.

The command reads and writes only `<root>/@<publishedid>/`. It does not read
`mod_sources.txt`, `mods.lock` or the Workshop cache.

Put the connection settings in `.uksfta/server.toml`. Git ignores the directory,
so the file stays local:

```toml
destination = "user@host"
port = 2022
root = "/"
identity = "/home/user/.ssh/id_ed25519"
host_key_policy = "yes"
known_hosts = "~/.ssh/known_hosts"
```

The file requires `destination` and `root`. The port defaults to 22. A flag
overrides the file.

#### Host key checks

<details>
<summary>The default policy and the accept-new option</summary>

The command checks the server host key on every `sftp` run. It passes
`StrictHostKeyChecking=yes` by default, so an unknown or changed key stops the
run. The key file is `~/.ssh/known_hosts`, unless `known_hosts` or
`--known-hosts PATH` names another file.

The command expands a leading `~/`. It refuses a relative path.

`--host-key-policy accept-new` accepts an unknown key once and keeps the entry. The
command prints its own notice when it adds a key, because the client does not
show its notice at `LogLevel=ERROR`.

The command never writes the key file itself. Under the default `yes` policy, a
missing or unreadable key file stops the run before the first connection.

The command also passes `UpdateHostKeys=no`, so a known key is not
replaced without a decision. A host key failure names the destination,
the key file and the remedy.

The native segmented path reads the same file. It checks the key before
authentication. It refuses an `@cert-authority` line and an `@revoked`
line, so those lines stop both paths.

</details>

#### Launch line and server.cfg checks

<details>
<summary>Launch line tokens and the server.cfg rules</summary>

Both checks are opt-in. They print one line per check on standard error
as a warning, and they do not change the exit code. `--strict-config`
makes a failing check part of the `Check` error, so the command exits 1.

The panel exposes no startup file over SFTP. Put a local copy of the launch line
in a file and pass `--launch-line FILE`, or set `launch_line_file` in
`server.toml`. The command does not read the line from the server.

It takes the last path component of each `-mod` and `-serverMod` entry and
compares it with `@<id>` byte for byte. A quoted token is one token, and an
unescaped `;` splits the value. A `-par` token hides the list, so the check
reports that it cannot verify the line.

The check prints one of these lines:

- `launch: @<id> is loaded`;
- `launch: @<id> is not in -mod` when only `-serverMod` names it;
- `launch: @<id> is not loaded`;
- `launch: folder case differs: @<found>`;
- `launch: -par hides the mod list`;
- `launch: loaded twice`.

`--server-cfg PATH`, or `server_cfg` in `server.toml`, reads that file once over
SFTP. The command uses an absolute path as written. It joins a relative path to
`--root`.

It allows a `..` component, because the file can sit above the workshop root.
The command reads no other server file and keeps no copy.

The check requires `verifySignatures = 2`. It requires `allowedFilePatching = 0`
when the key is present. An absent key is the engine default 0 and passes. A
non-empty `filePatchingExceptions[]` warns.

The command shows `equalModRequired` for information, and it never fails. A
missing `verifySignatures` fails, because the engine default is 0. A comment and
a quoted string cannot hide a setting.

</details>

#### JSON output and the doctor

<details>
<summary>The JSON fields and the doctor</summary>

`--json` writes one JSON object to standard output, and it writes nothing else
there. The human report and the warnings move to standard error, so a script can
parse the object. The object has `schema_version` (1), `pack`, `server`,
`status`, `counts`, `findings`, `actions`, `timings` and `result`.

`result` is `ok`, `check-failed` or `input-error`. The `server` object holds the
destination, port, root, known_hosts path, host key policy, buffer and requests.

It holds no credential, and it never includes the `server.cfg` contents. With
`--doctor`, the object has `mode`, `checks`, `free_space_bytes` and the timings
instead of a plan.

The segmented settings are additive. A default run adds no key, so its output is
byte-identical. When the feature is on, `server.segment` holds `enabled`,
`min_bytes`, `connections`, `chunk`, `resume` and `reconnects`, and a top-level
`uploads` array holds one entry per planned file.

Each entry has a `name` and a `mode` of `segmented` or `whole`. A segmented entry
also has `connections`. It gains the verified `sha256` after publication, so a
dry run marks the selected file without a hash.

`--doctor` is a read-only preflight. It prints the effective settings, checks the
known_hosts file, runs one `pwd` and listing batch at the pack folder, and asks
the server for free space. It does not plan, verify, upload or create a
directory.

It exits 0 when the connection and pack folder are usable, and 2 otherwise. A
server without `df` support reports unknown free space, which is not an error.
The doctor also prints `segment: not requested`, or the effective segmented
settings and the server extension line (`posix-rename` and `fsync`) when the
feature is on.

Every human run prints one `timing:` line on standard error with the
connect/list, deep, upload and total times in milliseconds. A failed run
reports the phases it completed. The same values are in the JSON
`timings` object.

Run the command from the pack repo root. It reads `meta.cpp` for the pack id and
name, and compares `.hemttout/release` with the server.

Use `--meta PATH` and `--local PATH` to change either. When `.hemttout/release`
is absent, the command uses `.hemttout/build`.

```bash
# Verify the pack against the server
uksfta server

# Compare SHA-256 as well as size
uksfta server --deep

# Print the plan and change nothing
uksfta server --upload --prune --dry-run

# Retry a failed upload up to three times
uksfta server --upload --retries 3

# Upload two files at a time
uksfta server --upload --jobs 2

# Check the launch line and the server config
uksfta server --launch-line launch.txt --server-cfg /server.cfg

# Make a failing opt-in check exit 1
uksfta server --server-cfg /server.cfg --strict-config

# Print one JSON object for a script
uksfta server --json

# Run the read-only preflight
uksfta server --doctor

# Raise the transfer window for a fast link
uksfta server --upload --buffer 261120 --requests 128

# Use the meter path with redirected output
uksfta server --upload --progress

# Prune an extra PBO that mods.lock lists
uksfta server --prune --force-prune
```

The build output must contain `addons/*.pbo`. The command reads every
file under the release root, at any depth. This layout is the usual one:

```text
<root>/@<publishedid>/addons/*.pbo
<root>/@<publishedid>/addons/*.bisign
<root>/@<publishedid>/keys/*.bikey
<root>/@<publishedid>/meta.cpp
<root>/@<publishedid>/mod.cpp
```

The command checks:

1. The remote `meta.cpp` `publishedid` equals the local id.
2. Every local PBO has a case-insensitive match in `addons/`, with the same size.
3. Every local `.bisign` pairs with its PBO. The command counts unsigned PBOs
   rather than listing them.
4. Every local `keys/*.bikey` exists with the same size.

5. `mod.cpp` exists with the same size.
6. Every local `.bisign` names an authority that a local `keys/*.bikey` carries,
   and, when the remote keys are readable, that the remote `keys/` carries too.
7. The command compares every other file at any depth by relative path, with the
   same size.

The walk skips four kinds of entry: any name that starts with `.`,
`*.biprivatekey`, `*.tmp` and `*.bak`, and symbolic links. It never
follows a link and never reads file contents. One warning gives the
count of skipped entries.

A remote-only file outside `addons/*.pbo` is `extra` and advisory. Prune
stays inside `addons/` and removes `.pbo` files only, so an included
file, a key, a signature, `meta.cpp` and `mod.cpp` are never removed.
`meta.cpp` and `mod.cpp` keep their own id and size checks.

The default comparison is size only. `--deep` hashes `addons/*.pbo` only.

A name that differs only by case is the same file. One warning gives the number of
such names, and uploads reuse the server name, so the command creates no case
duplicate.

A local PBO with no remote match is `missing`. A remote PBO with no local match
is `extra`.

Key matching uses the authority name inside the key file, not the file name. A
renamed `keys/*.bikey` still matches its signatures. A file name that differs
from the embedded authority gives a warning only.

The command reads the authority from the first 4096 bytes of a key. It does not
verify a signature, a hash or a PBO. A key that does not parse is a warning, and
its authority is unknown.

`--deep` compares the SHA-256 of each local PBO with the remote copy. It
reads the remote copy over the native SFTP session and hashes it as the
bytes arrive, so it writes no copy to disk. It is slow, because it reads
every PBO in the pack. Use it when a size match is not enough.

A setup the native session cannot reach, such as a password prompt or an
`ssh_config` proxy, falls back to downloading each PBO into a temporary
directory.

`--upload` puts every missing or stale file into the pack folder at its relative
path, including files outside `addons/` and `keys/`. It creates each directory
the uploads need, starting with the pack folder, before it sends the file. After
the upload the command lists the server again and re-runs the check.

</details>

#### Upload performance

An upload sends the smallest file first. A failure in the middle of a
batch therefore leaves the large files for the next attempt. Each file
uses `put -p`, so the server keeps the local modification time.

<details>
<summary>Buffers, retries, parallel jobs and progress</summary>

The transfer has two settings, and both pass to every `sftp` run,
including the verify phases and each parallel worker.

| Flag | Range | Default | Effect |
| --- | --- | --- | --- |
| `--buffer BYTES` | 4096 to 261120 | 32768 | The size of one write. |
| `--requests N` | 1 to 256 | 64 | The requests in flight. |

The in-flight data for one connection is about the buffer size times the request
count. Both values use memory at each end, so raise them only when the local link
is faster than the transfer. The server accepts at most 261120 bytes in one
write, which is 256 KiB minus 1024 bytes.

A 262144-byte buffer fails at once on the local client with
`Outbound message too long`.
Throughput is flat from 32 KB to 261 KB on the current line,
about 1.7 to 1.8 MB/s, because the link is saturated.
A larger buffer and more requests help a fast or high-latency link by reducing
round-trip stalls.

An upload attempt re-verifies the pack and retries only the files that are still
missing or stale. The default is 2 retries, which makes three attempts. Use
`--retries N` to change the count.

A value of 0 makes one attempt. The wait is 2 seconds after the first attempt,
then 4 seconds. Each attempt prints the attempt number, the file count, the
attempted bytes, the elapsed time and the rate in MB/s.

`--jobs N` uploads N files at the same time. The default is 1, which is a single
batch, and the maximum is 8. Each job runs in its own `sftp` process, so parallel
uploads need non-interactive authentication.

Use an agent or an identity file. A password prompt cannot serve several
workers.

The gain on the current line is small. Two streams gave 1.80 MB/s. One stream gave
1.70 to 1.81 MB/s.

Use more jobs only on a link with a high latency. Each attempt sends the files
that are still missing or stale again, and the command balances the files across
the workers by size.

An interactive upload shows one total bar, one row per planned file, and the
per-connection rows that the segmented file expands into. The rows read
`waiting`, `uploading` and `done`. The view is on by default when the output is a
terminal and `--jobs` is 1.

Use `--no-progress` to hide it. Use `--progress` when you redirect the output: it
prints one plain line every five seconds, with no escape codes.

Use `--progress-json` to write one JSON object per line on standard error, so
standard output stays clean for `--json`. The verify phases use `LogLevel=ERROR`,
so their output stays parseable.

An interrupted whole-file upload restarts the file from byte zero. The system
`sftp` path never resumes part of a file. In testing, `reput` trusted the remote
size and produced a corrupt file of the correct length.

Only a complete `put` gives a file that matches the local copy. The segmented
path does resume, and only from ranges the server acknowledged.

</details>

##### Segmented upload

`--segmented` sends one large file over several connections. The command picks the
largest planned upload at or above `--segment-min`. The other files use the
normal upload path.

The feature is off by default. The mode uses a key file or an agent, or one
password typed at the terminal with `--password-prompt`.

<details>
<summary>Flags, chunking, probe, resume and publish</summary>

| Flag | Range | Default | Effect |
| --- | --- | --- | --- |
| `--segmented` | on or off | off | Use the segmented path for one file. |
| `--segment-min BYTES` | up to 1 TiB | 64 MiB | The smallest file the segmented path takes. |
| `--segment-connections N` | 1 to 16 | 4 | The connections, and the ranges in flight. With `--segment-adaptive` it is the maximum. |
| `--segment-adaptive` | on or off | off | Scale the connections between 1 and the maximum. |
| `--segment-chunk BYTES` | 1 MiB to 64 MiB | automatic | The range size. It must be a 4096-byte multiple. It is fixed for the run. |
| `--limit BYTES` | at least 1 B/s | none | Cap the upload rate. A plain count is bytes per second. |
| `--preallocate` | on or off | on | Set the segmented temp file to its full length before the first range. |
| `--segment-probe` | on or off | on | Probe the link before segmenting a file of 256 MiB or more. |
| `--segment-fsync-every N` | 1 to 64 | 4 | Fsync every N completed ranges. |
| `--segment-resume POLICY` | auto or off | auto | Continue an earlier interrupted file. |
| `--segment-reconnects N` | 0 to 10 | 3 | The reconnects for one dead connection. |
| `--legacy-sftp` | on or off | off | Force the system `sftp` path for one release. |

The command sets the automatic chunk to one eighth of the file divided by the
connection count. It clamps the value to 1 to 16 MiB and rounds it down to a
4096-byte multiple.

The command chooses the chunk once, at plan time, and keeps it for the whole run.
The checkpoint indexes and the acknowledged prefix depend on fixed range
boundaries, so an adaptive run does not change the chunk.

For a file of 256 MiB or more, the command probes the link first. It moves small
files in the pack folder with hidden `.uksfta-probe-*` names: three 1 MiB uploads
on the system `sftp` path, one per job up to four, a native write of 4 MiB on one
connection, a native write of 4 MiB on two connections, and a native read-back of
4 MiB.

The system rate is the aggregate of the parallel uploads over the wall time, so
it models the `--jobs` path. The command estimates both paths and keeps the
whole-file path when the segmented estimate is not at least 10 percent better.
The decision line names the job count, the aggregate rate and both estimates.

A probe that cannot run keeps the segmented path with one warning. The system
probe never prompts: if it cannot authenticate, the command reports that and
keeps the segmented path, and the native measurements use the password you
typed.

`--password-prompt` asks once, before the probe and the transfer, while the
progress view is hidden. The probe never publishes, never uses a name Arma loads,
and removes its files on every exit path.

A hard kill can leave the probe files. The next probe overwrites and removes
them. `--no-segment-probe` turns it off.

Fsync now runs every fourth completed range by default
(`--segment-fsync-every`). The checkpoint can therefore record a range that was
not yet synced.

That is safe because a resume re-hashes the remote prefix, so a range lost after
a crash makes the prefix mismatch and the run restarts from a verified point. The
completion fsync, the publish hash and the equal-SHA-256 rule do not change.

The read-back hash uses its own 261120-byte buffer, not the 32 KiB
transfer buffer. The limit does not apply to the read-back.

The native transport sets a 16 MiB receive window and a 261120-byte packet cap,
and sizes its request buffer from `--requests`. The native path always writes in
261120-byte blocks, whatever `--buffer` says, because that is the server's
maximum packet.

`--buffer` keeps its safer 32 KiB default for the system `sftp` path. The probe's
native cases use the same block, so its numbers match the real transfer.

A segmented run reports `max`, `start` and `peak` connection counts in its
published line. Every adaptive change prints one line, such as
`segment: connections 4 -> 5`, and the log holds only changes.

`--limit` caps both upload paths. A plain number is bytes per second, and a `K`,
`M` or `G` suffix is 1024-based: `500K`, `2M`, `1.5M`. The system `sftp` path
passes the rate as `-l` in Kbit/s, rounded up.

The segmented path paces every connection through one shared limiter, so the
total stays at the rate. The limit covers upload writes only. The read-back hash
runs at full speed.

`--preallocate` sets the segmented temp file to its full length after it creates
the temp file and before the first range, so a shortfall appears before the
transfer. It is on by default. `--no-preallocate` turns it off. This client
exposes no fallocate extension, so the length comes from `set_len`.

A real failure, such as no space, stops the run with a clear message. A server
that does not support the request gives one warning and the upload continues.
The command leaves a resumed temp that already has its length untouched.

`--notify` sends one desktop notification when the run finishes, success or
failure. It runs `notify-send` on Linux and `osascript` on macOS. Any other
platform prints one warning and sends nothing.

The body names the pack id and name, the result, the file count, the bytes and the
elapsed seconds. A failure adds the sanitised error.

There is no notification for `--dry-run` or `--doctor`. `UKSFTA_NOTIFY_BIN`
replaces the notifier binary for tests: it receives the title and the body as two
arguments, and the command ignores its exit status.

`--segment-adaptive` starts at four connections, or the maximum when that is
smaller. The command starts one worker per allowed connection.

Only the workers below the active limit take ranges, and the others wait. A
monitor measures the acknowledged bytes over a window of about five seconds:

- The limit rises by one when the rate improves by at least 10 percent since
  the previous change, and the window had no retry.
- The limit falls by one after a retry, a connection failure, or a rate drop of
  at least 20 percent.
- Two windows pass after each change, so the limit cannot oscillate.

The command reads an extra connection that fails with a channel or resource error
as the server's session limit. That connection retires, the connections that work
carry the file, and the limit does not rise again in that run. The upload never
fails for a refused extra connection.

The view shows one row per allowed connection. A row above the active limit reads
`idle` and shows no meter.

A row that becomes active reads `waiting`, then shows its range. The command
keeps an idle row rather than removing it, so a worker that still holds its row
cannot panic.

The command writes the file to `<name>.uksfta-part`. The final name
changes only at publish. Each connection opens the temp file for writing
and seeks to its own range. No connection appends.

After the last range, the command downloads the whole temp file and hashes
it with SHA-256. It compares that hash with the local file before it
publishes. There is no flag to skip this read-back.

Publish runs one atomic rename. It uses `posix-rename@openssh.com` when the server
advertises it.

Otherwise it moves an old final file aside, renames the temp file, and removes the
old file. A rename that cannot run publishes nothing and exits 2.

An acknowledged range is a range whose write the server accepted and,
when the server advertises `fsync@openssh.com`, whose data reached the
disk. The checkpoint under `.uksfta/upload/<pack-id>/` records the
acknowledged prefix.

The acknowledged prefix is the number of bytes from offset zero that every range
below it has written and acknowledged. The command never trusts the remote size.

A size says how long a file is. It does not say which bytes are present.

Before a resume the command hashes the remote prefix and the local prefix. A match
continues from the acked prefix. A mismatch discards the temp file and the
checkpoint, and starts the file again.

A resumed run prints `segment: resuming at <percent>% (<bytes> of <total> bytes)`
before the first range. Its total bar starts at the acknowledged bytes.

After an upload the command prints a summary table for the files it sent: the name,
the bytes, the seconds, the rate, the SHA-256 and the remote path. The segmented
file gives exact seconds, rate and hash.

The command sends a whole file in a batch, so its seconds are its share of the
batch time and its rate is the batch rate.

Both values carry a `~` to mark the estimate. The command computes the local
SHA-256 for each whole file on four threads while the batch uploads, so the
pass costs no extra time. A total row gives the exact bytes, the elapsed
seconds and the average rate. A 0-byte file shows `0 B`, `0.0 s` and `-`.

The table goes to the human output only. With `--json` the same values appear in
each `uploads[]` entry as `bytes`, `seconds`, `mb_per_second`, `estimated`,
`sha256` and `remote`.

The read-back hash and the prefix hash answer the `reput` defect. An earlier tool
resumed at the remote size.

The server held a short file with a hole, so the result had the correct length
and wrong bytes. A prefix hash rejects that file.

The worker retries a failed range after 1 s, 2 s, 4 s, then 8 s. When a connection
dies, the command replaces only that connection and sends only its in-flight range
again.

`--segment-reconnects` sets how many times the command replaces one dead
connection. When every connection has given up, the run exits 2.

A failed run keeps the temp file and the checkpoint. The next run resumes
from the acked prefix. `--segment-resume off` discards any checkpoint and
starts the file again.

Before the first write the command asks for the free space. It needs twice
the file size. A shortfall names the free bytes and the needed bytes, and
exits 2. A server that cannot report free space gives one warning.

The command removes a stale temp file with a warning when it has no
checkpoint, or when it is more than a day old. It removes at most eight per
run. It never removes the current temp file.

A server that accepts only a password needs `--password-prompt` and a
terminal. Without a terminal, use the default `sftp` path.

A failed segment exits 2. The pack checks keep their codes: 1 for a check
failure and 2 for an input or environment failure. See Exit codes.

A config file may set `segment = true` and `segment_adaptive = true`. `--legacy-sftp` forces the system
`sftp` path for that run and conflicts with `--segmented`. The flag is an
escape hatch for one release.

The default transport stays the system `sftp` path. The flip to the native path is
a separate change.

That change keeps `--legacy-sftp` for one release and removes the flag after it.
The flip happens after all five criteria hold:

1. 20 consecutive live uploads of the 977 MB PBO with equal SHA-256
2. the Docker suite green on OpenSSH 9.6 and 10.6
3. 10 forced-kill resumes with equal hashes and no temp residue
4. host key and agent auth confirmed on the real panel
5. one release cycle with no defect report

`tests/throughput_live.rs` measures the segmented path against one stream
on a shaped bridge link. The link delays 60 ms and limits the rate to
40 Mbit. The test uploads a 128 MiB file twice and prints `single`,
`segmented` and `ratio`.

`tests/sftp_live.rs` adds shaped kill, loss-burst and repeated-resume cases. They
need `UKSFTA_SFTP_SHAPED=1` and a kernel that provides `sch_netem`.

Without that kernel they skip with a printed reason. The shaped container shapes
only its own interface, never the host network.

</details>

##### Measured throughput

The figures below come from a local OpenSSH container. They measure the
client and the server loop on one machine, not a WAN link. The container
varies between runs, so the table gives the median of five runs.

The 977 MB PBO on the segmented path, 8 connections, 8 MiB ranges,
`--buffer 261120 --requests 256` and the probe off:

| Version | Time | Rate |
| --- | --- | --- |
| 32 KiB block, 64 requests, 2 MiB window | 34.4 s | 29.80 MB/s |
| 261120-byte block, 256 requests, 16 MiB window | 14.7 s | 66.4 MB/s |

The best run of the second version took 14.4 s at 68.05 MB/s. Every run
published a file with the same SHA-256. A larger fsync interval gave no
reliable gain, so the default stays at every fourth range.

The full 15.46 GB pack, 114 PBOs, 8 jobs, probe on:

| Version | Time | Rate |
| --- | --- | --- |
| Probe era | 210.7 s | 75.03 MB/s |
| Current transport | 184.8 s | 83.77 MB/s |

The probe chose the segmented path for the 977 MB PBO and published it in
16.4 s at 59.47 MB/s.

The completion read-back reached 114.83 MB/s on one stream. A parallel read-back
needs to beat that by 20 percent, so the command keeps the single-stream hash. A
separate process that reopened the file read it at 15 to 16 MB/s.

That number does not match the in-run figure and has no cause yet. The in-run
figure is the read-back the transfer performs.

On the real server link the limit is the link. One stream gives 1.68 MB/s
and two streams give 1.80 MB/s, so the larger block and the extra
connections change little there. They help a fast or high-latency link.

##### Password prompt

`--password-prompt` reads one password from the controlling terminal for
the native segmented path. It is off by default.

The flag takes no value, so shell history cannot hold a secret. It
conflicts with `--legacy-sftp`.

<details>
<summary>Terminal use, failures and limits</summary>

There is no config key and no environment variable for the password. The
command never reads a file, an askpass helper, a keyring or the system
`sftp` binary for it.

The command opens `/dev/tty` (or `CONIN$` on Windows) before the host key
check and before any socket. Without a controlling terminal it exits 2
and names `--identity PATH`, ssh-agent and the default `sftp` path. The
prompt uses echo off.

The command makes one attempt. It prefers the password method, falls back to
keyboard-interactive when the server offers only that, and gives up after one
failure. A wrong password exits 2 with the destination, `(password)` and the
remaining methods.

The command never asks again. Server text in that message passes through the
terminal sanitiser.

An explicit prompt owns the credential. A wrong or empty password, or no accepted
method, is fatal for the run: the command exits 2 and publishes nothing.

It does not fall back to a key, an agent or the system `sftp` path for that file.
Other native failures, such as a dropped link or a failed publish, still fall
back to the system path.

The password lives in one string for the duration of the attempt. The
command does not write it to `.uksfta/`, the checkpoint, the JSON report
or the log.

The bytes are **not wiped**. `String` gives no way to overwrite the
buffer without `unsafe`, and this build adds no `zeroize` dependency. An
attacker who can read this process's memory is out of scope.

The live cases in `tests/password_live.rs` need a Linux PTY through
`script`. Without `script` they print a skip reason. The Windows runtime
is not covered by a live case.

</details>

The client also sends `ServerAliveInterval=15`, `ServerAliveCountMax=3`
and `ConnectTimeout=15`, so a dead link fails in seconds.

`--prune` removes remote PBOs that have no local match. It acts inside `addons/`
only, removes `.pbo` files only, and never removes `meta.cpp`, `mod.cpp`, a key, a
signature or a directory. The command keeps a remote-only PBO that `mods.lock`
lists, because the lock says the pack should contain it.

The command warns about it instead. Add `--force-prune` to remove it anyway.
Without `--prune` the command reports extras and changes nothing.

The command also reads `mods.lock` and `mod_sources.txt` from the repo root
when they exist. It uses them only for warnings:

- a PBO in `mods.lock` that is not in the local release warns;
- an enabled mod in `mod_sources.txt` with no `mods.lock` entry warns.

A missing file skips its check. Neither warning changes the exit code, and
neither file decides which server folder the command checks.

One line reports unsigned PBOs, for example `unsigned: 3 of 114 PBOs
have no signature (advisory)`. The line is advisory by default.
`--require-signatures` fails the command when the count is above zero, and
the error names the count.

`--dry-run` prints the plan and stops before any upload or removal. Each
plan line is the command that would run:

```text
plan: put /@3662499431/addons/brf_sumava.pbo from .hemttout/release/addons/brf_sumava.pbo
plan: rm /@3662499431/addons/projeta3019.pbo
```

The command stores no credential and prints none. Authentication is whatever
`sftp` supports: a key, an agent, or a prompt in the terminal. Use `identity` or
`--identity` to select a key.

The commands travel on standard input, so the prompt path stays available. The
command does not use the `-b` batch-file mode, because that mode suppresses the
prompt. The command reads and stores no password.

One `sftp` process runs per phase. The phases are the listing, the `meta.cpp` and
`mod.cpp` fetch, the upload and the prune. A password prompt therefore appears at
most once per phase. The deep phase reads over its own native session, so it
opens one more connection.

Each run uses `-R 64` and a 32 KB buffer. See Upload performance for the reasons
and the retry behaviour.

The output starts with the pack id and name, then the local and remote counts, the
status and one line per stale, missing or extra PBO.

A single line gives the unsigned count, and a single warning gives the count of
case-only name differences. The summary counts local PBOs, local-only PBOs and
remote-only PBOs.

The exit code is 0 when the pack matches or the run found only warnings. It is 1
for a missing or stale PBO, a key or `mod.cpp` problem, a `meta.cpp` id mismatch,
a missing pack folder, or any unsigned PBO under `--require-signatures`. It is 2
for a connection, usage or parse failure.

The command never uploads a file outside `<root>/@<publishedid>/`. It never
follows a symlink on the server, and it never recurses below `addons/` or
`keys/`.

### Dependencies

Every addon that the pack needs must be present, or listed as external.

<details>
<summary>How the addon closure is checked</summary>

`uksfta deps` checks the addon closure of a built pack. It reads every
`CfgPatches` class that each PBO in `addons/` provides, and every
`requiredAddons[]` entry that each PBO declares.

The command is local and offline. It does not read the Workshop cache,
`mods.lock` or the network.

The command reads `.hemttout/release` when that directory exists, else
`.hemttout/build`. Use `--local PATH` to choose another directory. A
missing directory is an input error and names the path.

Each requirement has one of three states:

- `provided`: a PBO in the pack provides the class.
- `external`: the external list names the class.
- `unaccounted`: nothing accounts for the class.

The external list holds the addon classes that the pack needs but does not provide.
These are base-game and DLC addons, and mods that the server or the launcher
supplies. Write one class on a line.

A line that starts with `#` is a comment. The default path is
`.uksfta/external-addons.txt`. Use `--external PATH` to choose another file.

```text
# Arma 3 base game
A3_Data_F
A3_Data_F_Loadorder

# Optional DLC
A3_Data_F_Jets
```

List a base-game or DLC addon once, whatever the number of PBOs that require
it.

Add `--mission PATH` to include the `addons[]` list of a mission PBO. The
`mission.sqm` entry must not use compression. A compressed or missing entry is an
input error that names the reason.

The command prints the count of provided classes, requirements and
unaccounted entries. It prints one line for each unaccounted entry, with
the requiring PBO and the addon class. `--suggest` prints the external
list format on standard output and writes no file.

The check is advisory. A run with an unaccounted entry exits 0, so a warning does
not stop a build. `--strict` exits 1 when the pack does not account for an entry.
A bad input, such as a missing build directory or a compressed mission, exits
2.

The command has four outcomes:

- The pack accounts for all entries. The command exits 0.
- An unaccounted entry without `--strict`. The command exits 0 and prints the
  entry.

- An unaccounted entry with `--strict`. The command exits 1.
- A bad input. The command exits 2 and names the path or the reason.

</details>

### Update checking

Every command checks for a newer release and prints a notice when one exists. The
command caches the result in the user cache directory, as
`uksfta/last-update-check`. On Linux this is `~/.cache/uksfta/`, and on Windows it
is `%LOCALAPPDATA%\uksfta\`.

A machine with no cache directory uses the system temp directory. The command
writes nothing to the repository.

A successful check stays fresh for 24 hours. A failed check stays fresh for 1
hour, so an offline machine retries once an hour and then continues at once. The
check records no diagnostic when it fails.

Set `UKSFTA_NO_UPDATE_CHECK` to any value to disable the automatic check.
`uksfta version` always checks, even with that variable set, and prints the
install one-liner for your platform.

## Reference

### Input format

`mod_sources.txt` names every mod in the pack. It has a TOML form and a
line-based form.

<details>
<summary>The mod_sources.txt formats</summary>

`mod_sources.txt` supports two formats. The TOML format (v2) adds metadata per
mod. The command writes it automatically when it first reads a legacy file, and
keeps a `.bak` backup:

```toml
version = 2

[[mods]]
id = "https://steamcommunity.com/sharedfiles/filedetails/?id=887302721"
name = "Boat Mod"
tags = ["vehicles"]

[[mods]]
id = "450814997"
name = "CBA_A3"

[[mods]]
id = "https://steamcommunity.com/sharedfiles/filedetails/?id=463939057"
role = "ignore"
enabled = false
```

Fields per mod:

- `id`: the Steam Workshop item ID, or the full Workshop page URL;
- `name`: display name, shown in sync output;
- `tags`: optional array of tags;
- `role`: `mod` (default) or `ignore`;
- `enabled`: set `false` to exclude without deleting the entry;
- `dependencies`: array of required Workshop IDs.

URLs are easier to verify at a glance, and the command accepts both forms. When
it migrates a legacy file, it writes ids as URLs so entries stay clickable.
`sync --resolve-deps` writes the discovered ids into `dependencies`
automatically, sorted and de-duplicated.

The command still accepts the legacy format (v1): one Workshop mod per line (URL
or bare ID), with an optional tag after `#`. It excludes mods under `[ignore]`,
such as the base dependency set that is not repacked.

```text
https://steamcommunity.com/sharedfiles/filedetails/?id=887302721 # Boat Mod
450814997 # CBA_A3

[ignore]
463939057 # ACE
```

</details>

### Output

`sync` writes:

- `addons/`: PBOs copied from the Workshop cache;
- `mods.lock`: JSON manifest of synced mods (files, name, tags, dependencies, last-updated timestamp).

The tool reads the Workshop cache and its `appworkshop_107410.acf` metadata
directly from your Steam libraries. It is offline by default. `investigate
--online`, `size --online`, and `sync --resolve-deps` use the network, as
does the automatic update check, which calls the GitHub API.

The `audit` command also flags orphan PBOs: files in `addons/` that belong to no
mod listed in `mod_sources.txt`. These are leftovers from removed mods, and you
can clean them up. It exits with a non-zero status when any expected PBO is
missing, so it can gate a build in CI.

### Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success, including "nothing to do". |
| 1 | A check failed. `verify` or `audit` found a missing PBO, `sync` found a required dependency absent from the cache, or `docs --check` found a missing or stale publish file. |
| 2 | Input or environment failure. A required file is missing or unreadable, or `mod_sources.txt` or `mods.lock` is unparseable. |

Network failures during `--online` commands are warnings and exit 0. Those
commands are best-effort. The one exception is `docs --online --check`: a
failed Workshop name lookup leaves the description stale, so `--check`
exits 1.

### Platforms

Steam library discovery is dynamic. The tool reads what Steam records and does
not guess paths:

- **Windows**: Steam's install path from the registry
  (`HKCU\Software\Valve\Steam`), then `libraryfolders.vdf` for all libraries.
- **Linux**: Steam's own `~/.steam/steam` symlink, then `libraryfolders.vdf`.

- **WSL**: the Windows Steam install via the registry (WSL interop), mapped to its
  `/mnt/<drive>` mount.

All library folders come from `libraryfolders.vdf`, so the tool finds extra
libraries (on any drive or mount) automatically.

## Licence

This project is licensed under the MIT Licence. See the `LICENSE` file.

## Maintainers

The UKSF Task Force Alpha Development Team maintains this project.
