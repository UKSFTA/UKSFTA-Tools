# UKSFTA Tools

**Unit-wide automation and DevOps infrastructure for UKSF Taskforce Alpha.**

This repository provides the core automation suite used to maintain, build, and deploy the UKSFTA mod ecosystem. It is intended to be used as a **Git Submodule** within all unit projects to ensure technical parity.

## 🛠 Features

- **Workspace Manager**: Centralized `dashboard`, `sync`, and `test` suite for project oversight.
- **Build Hardening**: Automated `publishedid` injection and Win32 timestamp normalization.
- **Mod Integrity**: Deep PBO inspection tool to detect source leaks and technical corruption.
- **HEMTT Integration**: Custom Rhai scripts for prefix auditing and version management.
- **Scaffolding**: Rapid project initialization with `Standard` and `CBA` templates.

## 🚀 Quick Start

1. **Integrate**: `git submodule add git@github.com:UKSFTA/UKSFTA-Tools.git .uksf_tools`
2. **Setup**: `python3 .uksf_tools/setup.py`
3. **Monitor**: `./tools/workspace_manager.py dashboard`

## 📋 Documentation

Detailed technical guides are available in the [**Wiki**](https://github.com/UKSFTA/UKSFTA-Tools/wiki) or the `/docs` folder:
- [**Architecture & Standards**](docs/ARCHITECTURE.md)
- [**Production Workflow**](docs/WORKFLOW.md)
- [**Troubleshooting**](docs/TROUBLESHOOTING.md)

## ⚖ License

This project is licensed under the **Arma Public License - Share Alike (APL-SA)**. See the `LICENSE` file for full details.
