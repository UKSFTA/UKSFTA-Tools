# UKSFTA Internal Tools

This directory contains the Python logic for the UKSFTA automation pipeline.

## Core Tools

| Script | Purpose |
| :--- | :--- |
| `manage_mods.py` | Workshop dependency manager and key purger. |
| `workshop_utils.py` | Shared helpers for workshop operations. |

## Usage

```bash
./tools/manage_mods.py [sync|status|keys|...]
```