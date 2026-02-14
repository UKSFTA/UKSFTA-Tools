# ⚔️ UKSFTA Platinum DevOps Suite v1.4.0

The **UKSFTA Platinum DevOps Suite** is a high-fidelity unit management and automation toolkit designed for professional Arma 3 development. It provides a "Zero Trust" infrastructure that ensures project stability, security, and performance across the entire unit workspace.

## 🚀 Key Features

### 🌐 Workspace Operations
- **`status`**: Instant git status summary for every unit repository.
- **`sync`**: Automated Workshop dependency synchronization and lockfile management.
- **`update`**: One-click propagation of latest DevOps tools to all unit projects.
- **`self-update`**: Keep your local toolkit synchronized with the master repository.

### 🧠 Forensic Intelligence (Phase 3)
- **`classify-asset`**: Heuristic engine that identifies P3D categories (Uniform, Vest, Weapon) via internal metadata.
- **`diff-models`**: Binary-level comparison of P3D assets, reporting changes in Mass, LODs, and VFS Links.
- **`workshop-info`**: Query live versions, sizes, and timestamps directly from Steam.
- **`modlist-size`**: Calculate the total data footprint of any Arma 3 modlist.

### 🛡️ Assurance & Quality (Phase 1 & 4)
- **`audit`**: Master command running the full suite of health and security checks.
- **`audit-lods`**: Deep-scan P3Ds for missing Shadow Volume or Geometry LODs.
- **`audit-vfs-links`**: Detect "external leakage" and dead texture/material paths in assets.
- **`rebin-guard`**: Pre-binarization safety check to ensure assets are stable for production builds.
- **HEMTT Hook**: Automated forensic audit that halts the build cycle if asset defects are detected.

### 🏗️ Asset Ingestion & Porting (Phase 2 & 5)
- **`import-wizard`**: One-click ingestion of external assets with automated sanitization and refactoring.
- **`remap-advanced`**: Bulk-replace texture and material paths inside binarized P3D files.
- **RVMAT Refactoring**: Automated path normalization inside material files.
- **Config Generation**: Auto-generate `config.cpp` boilerplates based on forensic classification.

### 🛰️ Distributed DevOps (Remote Node Management)
- **`remote setup`**: Automated onboarding of a new VPS node.
- **`remote run`**: High-speed task delegation to the unit's remote gigabit backbone.
- **`remote monitor`**: High-fidelity resource reporting for all distributed nodes.

## 💻 Developer Experience (DX)
- **Git Hooks**: Local pre-commit guards to block security leaks and syntax errors.
- **VS Code Integration**: One-click task menu for all common dev actions.
- **Rich CLI**: Beautiful, high-performance terminal interface using the `rich` library.

---

## 🛠 Getting Started

1. **Prerequisites**: Ensure you have `python3`, `git`, `hemtt`, `steamcmd`, and `ansible` installed.
2. **Setup**: Run `./tools/workspace_manager.py check-env` to verify your local environment.
3. **Usage**: Run `./tools/workspace_manager.py help` to see the full command suite.

---

### Maintained by the UKSF Taskforce Alpha Development Team
