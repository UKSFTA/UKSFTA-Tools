# UKSFTA Platinum DevOps Suite - Engineering Manual

**Pipeline Status**: 🛡️  Sovereign Sync Active

 
## 1. Unit Architecture
The UKSFTA ecosystem utilizes a multi-repository structure managed by a centralized suite of automation tools. This ensures technical synchronization and "Diamond Grade" production standards across all unit assets.

 
## 2. Build Orchestration (`build.sh`)
The master build script implements **Solid Staging** standards:
- **Forensic Audit**: Executes `asset_auditor.py` pre-build to detect orphaned assets or VFS naming violations.
- **Physical Staging**: Bypasses Linux/Proton symlink restrictions by physically copying PBOs and metadata into the Arma 3 root (`@UKSFTA-...`).
- **Unit Hub Consolidation**: Automatically packages releases into standardized ZIP formats and moves them to the `all_releases` central hub.

 
## 3. Technical Hygiene Standards
 
### Virtual Filesystem (VFS)
- **Diamond Prefix**: All addons MUST use the `z\uksfta\addons\<addon_name>` prefix.
- **Header Isolation**: Core macros and versioning are managed via `script_component.hpp` and `script_version.hpp`.

 
### Commit Governance
- **GPG Signing**: Every commit MUST be signed with a verified GPG key.
- **Quality Guard**: Pre-commit hooks execute SQF validation and Markdown linting.

 
## 4. Headless Validation Suite
Located in `UKSFTA-Maps/tests/`, this suite allows for logic verification without launching Arma 3.

 
### Tools
- **sqflint**: Automated static analysis for syntax and variable usage.
- **sqfvm**: A virtual machine for executing SQF logic in a headless environment.
- **HEMTT Check**: Verifies project structure and rapification readiness.

 
### Execution
Run the master audit via:
```bash
cd UKSFTA-Maps/tests
./run_tests.sh
```
This script orchestrates:
1.  **Build Integrity**: (HEMTT audit)
2.  **Logic Stress Test**: Executes the weather state machine for 1,000 cycles.
3.  **Matrix Audit**: Verifies cross-mod compatibility (Vanilla vs ACE vs TFAR).
4.  **Performance Benchmark**: Reports average instruction-count per cycle for core functions.

 
## 5. Deployment Pipeline (CI/CD)
GitHub Actions are configured to:
1.  **Lint**: Audit all SQF and Markdown files.
2.  **Scan**: Perform Security/CodeQL audits for secrets and vulnerabilities.
3.  **Deploy**: Build and publish the Intelligence Portal (COP) to GitHub Pages with high-fidelity map tiles and credential injection.
