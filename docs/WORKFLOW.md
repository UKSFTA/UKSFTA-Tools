# Production Workflow

This is the standard "Runbook" for the UKSFTA mod lifecycle.

## 1. Scaffolding a New Project
Never create a mod folder manually. Use the scaffold tool to ensure VFS compliance:
```bash
# Standard mod (models/textures)
./new_project.sh UKSFTA-MyMod standard

# Logic mod (CBA-macros/Functions)
./new_project.sh UKSFTA-MyScript cba
```

## 2. Dependency Management
Always keep Workshop dependencies synchronized. This process automatically purges illegal signing keys:
```bash
./tools/workspace_manager.py sync
```

## 3. The Build-Audit Loop
Before pushing any code, run the full validation and build suite:
1. **Lint:** `./tools/workspace_manager.py test` (Runs SQFLint, hemtt check, and Custom Validators).
2. **Build:** `./tools/workspace_manager.py build`.
3. **Audit:** `./tools/workspace_manager.py audit-build` (Verified PBO headers and leak protection).

## 4. Final Release & Publication
1. **Package:** `./tools/workspace_manager.py release` (Creates ZIPs with hardened metadata).
2. **Preview:** `./tools/workspace_manager.py publish --dry-run` (Visual mock of the Steam Workshop page).
3. **Push:** `./tools/workspace_manager.py publish` (Uploads to Steam).
