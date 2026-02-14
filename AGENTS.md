# AI Agent Manual (AGENTS.md)

This file provides critical context for AI assistants maintaining the UKSFTA workspace.

## 🤖 Maintenance Protocol

When performing tasks in this repo, the agent MUST:

1. **Plan First:** Propose a technical plan before writing code.
2. **Linting:** Run `./tools/workspace_manager.py test` before concluding any change.
3. **Commit Signing:** ALL commits must be GPG signed (`-S` flag).
4. **Submodule Parity:** After updating tools, run `./tools/workspace_manager.py update` to sync project submodules.

## 🛠 Tooling Logic

- **`build.sh`**: Our primary wrapper. NEVER call `hemtt release` directly; always go through this wrapper to ensure metadata injection.
- **`publishedid`**: This must be treated as a strict requirement. If a project has an ID in `.hemtt/project.toml`, it MUST be in `meta.cpp`.

## 📁 Source of Truth

Consult the following before suggesting architectural changes:

- `docs/ARCHITECTURE.md`: VFS and Logic/Asset separation rules.
- `docs/WORKFLOW.md`: The 4-step pipeline (Sync -> Test -> Build -> Release).

## 🚀 Modernized Toolset Integration (2026-02-14)

### P3D Debinarizer (v1.2.0)
- **Status:** Integrated into Platinum DevOps Suite.
- **HEMTT Alignment:** Fully compatible with HEMTT project structures and prefix mapping.
- **Integrated Commands:**
  - `debinarize`: High-fidelity ODOL to MLOD conversion with path fixing.
  - `migrate-prefix`: Automated project-wide prefix remapping (uses .hemtt/project.toml).
  - `list-models`: HEMTT-aware asset inventory and metadata reporting.
- **Distribution:** Binary deployed at `bin/linux-x64/debinarizer`, Python wrapper in `tools/p3d_debinarizer.py`.
- **Reverse Engineering:** First comprehensive documentation of ODOL v73 format in `docs/ODOL_v73_SPEC.md` (in debinarizer repo).
