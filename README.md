# UKSFTA-Tools

Shared tooling for the UKSF Task Force Alpha Arma 3 workspace, written in Rust.

## Usage

Run from a mod repository root (a directory containing `mod_sources.txt` and `addons/`).

```bash
# Preview what a sync would change
uksfta sync --dry-run

# Copy PBOs from the local Steam Workshop cache into addons/, write mods.lock
uksfta sync

# Generate an Arma 3 launcher modlist for missing mods
uksfta sync --modlist

# Custom output path for the modlist
uksfta sync --modlist --modlist-path ./my-modlist.html

# Also resolve dependencies from Steam Workshop pages
uksfta sync --modlist --resolve-deps

# Show which PBOs came from which Workshop mod
uksfta identify

# Confirm all locked PBOs are present
uksfta verify

# Audit each mod's PBOs against addons/ (present/missing per PBO)
uksfta audit

# Show only missing PBOs and per-mod counts
uksfta audit --missing-only

# Compare mods.lock timestamps against the Workshop cache
uksfta updates

# Import mods from an Arma 3 launcher modlist HTML file
uksfta import ./my-modlist.html

# Preview what would be imported without changing mod_sources.txt
uksfta import ./my-modlist.html --dry-run
```

## Input Format

`mod_sources.txt` supports two formats. The TOML format (v2) adds metadata
per mod and is written automatically when a legacy file is first read (a
`.bak` backup is kept):

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

- `id` — Steam Workshop item ID, or the full Workshop page URL. URLs are
  easier to verify at a glance; both forms are accepted. When a legacy
  file is migrated, ids are written as URLs so entries stay clickable.
- `name` — display name, shown in sync output
- `tags` — optional array of tags
- `role` — `mod` (default) or `ignore`
- `enabled` — set `false` to exclude without deleting the entry
- `dependencies` — optional array of Workshop IDs

The legacy format (v1) is still accepted: one Workshop mod per line (URL
or bare ID), with an optional tag after `#`. Mods under `[ignore]` are
excluded (for example the base dependency set that is not repacked).

```text
https://steamcommunity.com/sharedfiles/filedetails/?id=887302721 # Boat Mod
450814997 # CBA_A3

[ignore]
463939057 # ACE
```

## Output

- `addons/` — PBOs copied from the Workshop cache
- `mods.lock` — JSON manifest of synced mods (files, name, tags, dependencies, last-updated timestamp)

The tool reads the Workshop cache and its `appworkshop_107410.acf` metadata
directly from your Steam libraries. It performs no Steam API calls.

The `audit` command also flags orphan PBOs: files in `addons/` that belong
to no mod listed in `mod_sources.txt`. These are leftovers from removed
mods and can be cleaned up. It exits with a non-zero status when any
expected PBO is missing, so it can gate a build in CI.

## Missing mods

When a mod in `mod_sources.txt` is not present in any local Workshop
cache, `sync` reports it with its name and a direct link to the Steam
Workshop page, then lists a set of `steam://url/CommunityFilePage/...`
deep links so each mod can be subscribed to in Steam. Once subscribed
and downloaded, re-run `uksfta sync` to pull the PBOs into `addons/`.

Use `--modlist` to generate an Arma 3 launcher preset file (HTML) that
lists all missing mods. Open the file in the launcher to batch-subscribe
to every missing mod at once. The output defaults to `missing-mods.html`;
override with `--modlist-path`.

Add `--resolve-deps` to also fetch each missing mod's Workshop page and
discover dependencies not listed in `mod_sources.txt`. Discovered
dependencies are reported in the missing-mod warning and included in the
generated modlist. This requires a network connection and adds a
one-second delay per mod for rate limiting.

## Importing a modlist

Use `import` to add mods from an Arma 3 launcher preset file (HTML) to
`mod_sources.txt`:

```bash
# Add all Steam mods from a shared modlist
uksfta import ./my-modlist.html

# Preview first — prints what would be added, changes nothing
uksfta import ./my-modlist.html --dry-run
```

Each Steam mod is appended as `{id} # {name}`. Mods already present in
`mod_sources.txt` (including the `[ignore]` section) are skipped. Local
mods without a Workshop ID are skipped with a warning. New entries are
inserted before the `[ignore]` section if one exists.

## Platforms

Steam library discovery is dynamic — the tool reads what Steam records,
it does not guess paths:

- **Windows**: Steam's install path from the registry
  (`HKCU\Software\Valve\Steam`), then `libraryfolders.vdf` for all
  libraries.
- **Linux**: Steam's own `~/.steam/steam` symlink, then
  `libraryfolders.vdf`.
- **WSL**: the Windows Steam install via the registry (WSL interop),
  mapped to its `/mnt/<drive>` mount.

All library folders come from `libraryfolders.vdf`, so extra libraries
(on any drive or mount) are found automatically.

## Installation

### Windows (recommended)

Run this in PowerShell:

```powershell
irm https://github.com/UKSFTA/UKSFTA-Tools/releases/latest/download/install.ps1 | iex
```

This downloads `uksfta.exe`, verifies its SHA256 checksum against the
release's `SHA256SUMS`, installs it under `%LOCALAPPDATA%\Programs\uksfta`,
and adds it to your user PATH. Open a new terminal and run `uksfta --help`
to verify.

### Manual download

Download the binary for your platform from the [Releases
page](https://github.com/UKSFTA/UKSFTA-Tools/releases) and put it on your
`PATH`:

```bash
# Linux
chmod +x uksfta
sudo mv uksfta /usr/local/bin/uksfta

# Windows
# uksfta.exe from the release, rename to uksfta.exe if needed and add to PATH
```

Each release also includes a `SHA256SUMS` file so you can verify the
binary you downloaded.

Or build from source: `cargo build --release` (Linux, macOS) /
`cargo build --release --target x86_64-pc-windows-msvc` (Windows, matches
the published release binary).

## Licence

This project is licensed under the MIT Licence. See the `LICENSE` file.

### Maintained by the UKSF Taskforce Alpha Development Team
