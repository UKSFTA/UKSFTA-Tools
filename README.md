# UKSFTA-Tools

Shared tooling for the UKSF Task Force Alpha Arma 3 workspace, written in Rust.

## Usage

Run from a mod repository root (a directory containing `mod_sources.txt` and `addons/`).

```bash
# Preview what a sync would change
uksft sync --dry-run

# Copy PBOs from the local Steam Workshop cache into addons/, write mods.lock
uksft sync

# Show which PBOs came from which Workshop mod
uksft identify

# Confirm all locked PBOs are present
uksft verify

# Compare mods.lock timestamps against the Workshop cache
uksft updates
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

## Licence

This project is licensed under the MIT Licence. See the `LICENSE` file.

### Maintained by the UKSF Taskforce Alpha Development Team
