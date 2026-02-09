# UKSFTA DevOps Guide

This document outlines the technical architecture and automation standards for the UKSFTA mod development environment.

## 🏗 Workspace Architecture

The workspace is organized into functional layers:
- **Logic (Brain):** `UKSFTA-Scripts`. Standalone PBO project containing all SQF functions. Automatically registered in the engine's Function Library.
- **Assets (Body):** `UKSFTA-Mods`, `UKSFTA-Maps`. PBO projects containing 3D models, textures, and terrains.
- **Infrastructure (Nervous System):** `UKSFTA-Tools`. Centralized build wrappers, validators, and management scripts.

## 🚀 Key Automation Tools

### 1. Workspace Manager (`workspace_manager.py`)
Centralized control for multi-repo environments.
- `dashboard`: Real-time health and sync status overview.
- `sync`: Harmonizes Workshop dependencies and purges external keys.
- `test`: Full-suite validation (pytest, sqflint, hemtt check, UKSFTA custom validators).
- `audit-build`: Deep inspection of built PBOs for corruption or source leaks.
- `clean`: Purges all build caches (`.hemttout`).

### 2. Build Wrapper (`build.sh`)
Hardened wrapper for HEMTT that enforces Unit Standards:
- **No-Sign Policy:** Disables mod signing by default to prevent Launcher corruption.
- **Metadata Injection:** Automatically injects the correct `publishedid` into `meta.cpp` from `project.toml`.
- **Timestamp Normalization:** Sets Win32 timestamps to current time for Launcher consistency.

### 3. Mod Integrity Checker (`mod_integrity_checker.py`)
Performs pre-flight checks on PBOs:
- Validates PBO headers (`0x00` null prefix).
- Detects loose source file leaks (`.sqf`, `.paa`, etc).
- Identifies illegal characters in filenames that crash Linux servers.

## 🛠 Scripting Standards

### CBA Framework Integration
All logic projects use the `cba` template, providing:
- `script_macros.hpp`: Links to standard CBA macros (`GVAR`, `QUOTE`).
- `CfgFunctions`: Automatic registration of `fnc_*.sqf` files as `UKSFTA_fnc_Name`.

### Validation Pipeline
1. **Local:** Run `./tools/workspace_manager.py test` before pushing.
2. **CI:** GitHub Runners execute `lint.yml` and `build.yml` on every PR.

## 🔧 Mikero Tools Integration
UKSFTA environments use version **0.10.13+** of Mikero's tools. Use the `depbo` command for a quick cheatsheet of all available binary operations.
