# UKSFTA-Tools

Shared tooling for the UKSF Task Force Alpha Arma 3 workspace, written in Rust.

## Usage

Run from a mod repository root (a directory containing `mod_sources.txt` and `addons/`).

```bash
# Preview what a sync would change
uksfta sync --dry-run

# Copy PBOs from the local Steam Workshop cache into addons/, write mods.lock
uksfta sync

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
```

## Input Format

`mod_sources.txt` — one Workshop mod per line (URL or bare ID), with an
optional tag after `#`. Mods under `[ignore]` are excluded (for example the
base dependency set that is not repacked).

```text
https://steamcommunity.com/sharedfiles/filedetails/?id=887302721 # Boat Mod
450814997 # CBA_A3

[ignore]
463939057 # ACE
```

## Output

- `addons/` — PBOs copied from the Workshop cache
- `mods.lock` — JSON manifest of synced mods (files, name, dependencies, last-updated timestamp)

The tool reads the Workshop cache and its `appworkshop_107410.acf` metadata
directly from your Steam libraries. It performs no Steam API calls.

The `audit` command also flags orphan PBOs: files in `addons/` that belong
to no mod listed in `mod_sources.txt`. These are leftovers from removed
mods and can be cleaned up. It exits with a non-zero status when any
expected PBO is missing, so it can gate a build in CI.

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

Download the binary for your platform from the [Releases
page](https://github.com/UKSFTA/UKSFTA-Tools/releases) and put it on your
`PATH`:

```bash
# Linux
chmod +x uksfta-linux
sudo mv uksfta-linux /usr/local/bin/uksfta

# Windows
# rename uksfta-windows.exe to uksft.exe and add to PATH
```

Or build from source: `cargo build --release` (Linux, macOS) /
`cargo build --release --target x86_64-pc-windows-gnu` (Windows).

## Licence

This project is licensed under the MIT Licence. See the `LICENSE` file.

### Maintained by the UKSF Taskforce Alpha Development Team
