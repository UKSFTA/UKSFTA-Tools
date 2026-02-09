# UKSFTA Internal Tools

This directory contains the Python-based logic for the UKSFTA automation pipeline.

### Core Tools

| Script | Purpose |
| :--- | :--- |
| `workspace_manager.py` | Central hub for multi-project sync, audit, and status. |
| `mod_integrity_checker.py` | Deep inspection of PBO headers and structure. |
| `manage_mods.py` | Workshop dependency manager and key purger. |
| `fix_timestamps.py` | Normalizes `meta.cpp` metadata and Win32 timestamps. |
| `release.py` | Orchestrates versioning, building, and Steam uploading. |

### Validation Suite

These scripts are run automatically by `workspace_manager.py test` and GitHub Actions:
- `config_style_checker.py`: Enforces standard formatting in `.cpp` files.
- `sqf_validator.py`: Catch basic syntax and bracket errors in SQF.
- `stringtable_validator.py`: Validates XML structure and `AFM` naming conventions.
- `return_checker.py`: Verifies SQF return types match documentation blocks.
- `search_unused_privates.py`: Identifies local variables not declared as private.

### Usage

Most tools are designed to be run via the **Workspace Manager**:
```bash
./tools/workspace_manager.py [status|sync|build|release|test|audit-build|cache|clean]
```
