# UKSFTA-Tools

Centralized automation and DevOps infrastructure for UKSF Taskforce Alpha mod projects. This repository is intended to be used as a **Git Submodule** within other unit projects.

## 🛠 Features

- **Mod Manager (`manage_mods.py`)**: Automates Workshop dependency syncing, dependency resolution, and smart cleanup of orphaned PBOs/Keys.
- **Release Tool (`release.py`)**: Handles version bumping, HEMTT building, changelog generation, Steam Workshop uploading, and GitHub Release creation.
- **HEMTT Integration**: Shared scripts and hooks for versioning and artifact management.
- **Validation**: Includes SQF, Config, and Stringtable validators.

## 🚀 Integration

To add these tools to a project:

1. **Add Submodule**:
   ```bash
   git submodule add git@github.com:UKSFTA/UKSFTA-Tools.git .uksf_tools
   ```

2. **Run Setup**:
   ```bash
   python3 .uksf_tools/setup.py
   ```
   *This creates the necessary symlinks in `tools/` and `.hemtt/`.*

## 📋 Mod Management

Add Steam Workshop links to `mod_sources.txt`. To exclude specific dependencies:

```text
[ignore]
https://steamcommunity.com/sharedfiles/filedetails/?id=450814997 # CBA_A3
```

## ⚖ License

Licensed under the MIT License. See LICENSE for details.
