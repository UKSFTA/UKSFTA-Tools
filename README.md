# UKSFTA DevOps Suite

Automation toolkit for Arma 3 mod development. Provides a pipeline for
project stability, security, and performance across the unit workspace.

## Workspace Operations

- **`status`**: Git status summary for every unit repository.
- **`sync`**: Workshop dependency synchronisation and lockfile management.
- **`update`**: Propagate latest DevOps tools to all unit projects.
- **`self-update`**: Synchronise local toolkit with the master repository.

## Forensic Intelligence

- **`classify-asset`**: Heuristic engine that identifies P3D categories
  (Uniform, Vest, Weapon) via internal metadata.
- **`diff-models`**: Binary-level comparison of P3D assets, reporting changes
  in Mass, LODs, and VFS Links.
- **`workshop-info`**: Query live versions, sizes, and timestamps from Steam.
- **`modlist-size`**: Calculate the total data footprint of any Arma 3 modlist.

## Assurance and Quality

- **`audit`**: Full suite of health and security checks.
- **`audit-lods`**: Deep-scan P3Ds for missing Shadow Volume or Geometry LODs.
- **`audit-vfs-links`**: Detect external leakage and dead texture/material
  paths in assets.
- **`rebin-guard`**: Pre-binarisation safety check to ensure assets are stable
  for production builds.
- **HEMTT Hook**: Automated forensic audit that halts the build cycle if
  asset defects are detected.

## Asset Ingestion and Porting

- **`import-wizard`**: One-click ingestion of external assets with automated
  sanitisation and refactoring.
- **`remap-advanced`**: Bulk-replace texture and material paths inside
  binarised P3D files.
- **RVMAT Refactoring**: Automated path normalisation inside material files.
- **Config Generation**: Auto-generate `config.cpp` boilerplates based on
  forensic classification.

## Distributed DevOps

- **`remote setup`**: Onboarding of a new VPS node.
- **`remote run`**: Task delegation to the unit's remote infrastructure.
- **`remote monitor`**: Resource reporting for all distributed nodes.

## Developer Experience

- **Git Hooks**: Pre-commit guards to block security leaks and syntax errors.
- **VS Code Integration**: Task menu for all common dev actions.
- **Rich CLI**: Terminal interface using the `rich` library.

## Getting Started

1. **Prerequisites**: `python3`, `git`, `hemtt`, `steamcmd`, `ansible`.
2. **Setup**: Run `./tools/workspace_manager.py check-env` to verify your
   local environment.
3. **Usage**: Run `./tools/workspace_manager.py help` to see the full command
   suite.

## Licence

This project is licensed under the MIT Licence. See the `LICENSE` file.

---

Maintained by the UKSFTA Development Team.
