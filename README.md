# UKSFTA-Tools

Centralized automation and DevOps infrastructure for UKSF Taskforce Alpha mod projects. This repository is intended to be used as a **Git Submodule** within other unit projects.

## 🛠 Features

- **Workspace Manager**: Centralized `dashboard`, `sync`, and `test` suite for managing all unit projects.
- **Build Hardening**: Automated `publishedid` injection and timestamp normalization to prevent Launcher corruption.
- **Mod Integrity**: Deep PBO inspection tool to catch source leaks and header errors.
- **HEMTT Integration**: Shared scripts and hooks for versioning, prefix checking, and artifact management.
- **Scaffolding**: Rapid project creation with `Standard` and `CBA` templates.

## 🚀 Quick Start

1. **Add Submodule**:
   ```bash
   git submodule add git@github.com:UKSFTA/UKSFTA-Tools.git .uksf_tools
   ```

2. **Run Setup**:
   ```bash
   python3 .uksf_tools/setup.py
   ```

3. **Manage Workspace**:
   ```bash
   # From the tools repo root:
   ./tools/workspace_manager.py dashboard
   ./tools/workspace_manager.py test
   ```

## 📋 Documentation

For detailed technical standards, build pipeline logic, and troubleshooting, see the [DevOps Guide](docs/devops_guide.md).

## ⚖ License

Licensed under the MIT License. See LICENSE for details.
