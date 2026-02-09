# UKSFTA-Tools

Centralized automation and DevOps infrastructure for UKSF Taskforce Alpha mod projects. This repository is intended to be used as a **Git Submodule** within other unit projects.

## 🛠 Features

- **Workspace Manager**: Centralized `dashboard`, `sync`, and `test` suite for managing all unit projects.
- **Build Hardening**: Automated `publishedid` injection and timestamp normalization to prevent Launcher corruption.
- **Mod Integrity**: Deep PBO inspection tool to catch source leaks and header errors.
- **HEMTT Integration**: Shared scripts and hooks for versioning, prefix checking, and artifact management.
- **Scaffolding**: Rapid project creation with `Standard` and `CBA` templates.

## 🚀 Quick Start

1. **Add Submodule**: `git submodule add git@github.com:UKSFTA/UKSFTA-Tools.git .uksf_tools`
2. **Run Setup**: `python3 .uksf_tools/setup.py`
3. **Open Dashboard**: `./tools/workspace_manager.py dashboard`

## 📋 Technical Documentation

Explore our in-depth DevOps guides:
- [**Architecture & Standards**](docs/ARCHITECTURE.md): The "Logic vs. Assets" model and VFS rules.
- [**Environment Setup**](docs/SETUP.md): Zero-to-hero guide for Linux, SteamCMD, and Mikero tools.
- [**Production Workflow**](docs/WORKFLOW.md): Step-by-step "Runbook" from scaffold to Steam Workshop.
- [**Troubleshooting**](docs/TROUBLESHOOTING.md): Fixes for Launcher corruption and Btrfs redownload loops.

## ⚖ License

Licensed under the MIT License. See LICENSE for details.
