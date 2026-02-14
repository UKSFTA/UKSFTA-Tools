# 🛠 UKSFTA Platinum DevOps Suite - Roadmap

## 🎯 Global Standards
- [ ] **100% Cross-Compatibility:** All tools must run natively on Linux-x64 and Windows-x64.
- [ ] **Path Normalization:** Automate `` vs `/` conversion for Arma VFS vs System FS.
- [ ] **Atomic Operations:** Ensure file writes are atomic to prevent asset corruption during build failures.
- [ ] **GPG Signed:** All tool updates must be cryptographically signed.

---

## Phase 1: Assurance Engine (v1.3.0)
*Status: Complete*

- [x] **`audit-lods`**:
    - Detect missing Shadow Volume LODs (Performance).
    - Verify Geometry/PhysX LOD integrity (Server Stability).
    - Report vertex count density across all resolutions.
- [x] **`audit-vfs-links`**:
    - Deep-scan `.p3d` textures (`.paa`) and materials (`.rvmat`).
    - Validate existence of all links in workspace or dependencies.
    - Flag "external leakage" (references to non-unit paths).
- [ ] **`optimize-dashboard`**:
    - Visual CLI report of project "weight" (Disk size vs Memory footprint).

## Phase 2: Migration & Refactor Suite
*Status: Complete*

- [x] **`remap-advanced`**:
    - Bone remapping (Map legacy skeleton to UKSFTA standard).
    - Material swapping (Bulk replace legacy shaders with optimized versions).
- [x] **`manage-proxies`**:
    - CLI-based proxy injection (Inject rails/lasers to groups of models).
    - Proxy sanitization (Remove orphaned attachment points).
- [x] **`rebin-guard`**:
    - Pre-binarization check for "Micro-gap" geometry.
    - Ensure debinarized MLODs meet standard BI tool requirements.

## Phase 3: Forensic Intelligence
*Status: Complete*

- [x] **`diff-models`**:
    - Binary-level comparison of ODOL versions.
    - Human-readable changelog generation for asset updates.
- [x] **`auto-classifier`**:
    - Use forensic patterns to identify asset categories (Uniform, Vest, Weapon).
    - Auto-generate Workshop tags and manifest entries.

## Phase 4: HEMTT Deep Integration
*Status: Complete*

- [x] **`pre_build` Forensic Hook**:
    - Execute JIT (Just-In-Time) debinarization for third-party submodules.
    - Dynamic path re-prefixing during the HEMTT build cycle.
- [ ] **Unit Signer Integration**:
    - Seamless signing of debinarized artifacts within the HEMTT pipeline.

## Phase 5: Asset Ingestion & Porting (v1.4.0)
*Status: Complete*

- [x] **`import-wizard`**:
    - Automated "One-Click" porting of legacy/external assets.
    - Recursive filename sanitization (lower_case + snake_case).
    - **RVMAT Refactoring**: Bulk path remapping in material files.
    - **Boilerplate Generator**: Auto-generate `config.cpp` entries via asset classification.
- [ ] **Unit-Wide Normalization**:
    - Bulk migrate all existing `UKSFTA-*` repositories to the new VFS prefix standard.

## Phase 6: Global Path Normalization (v1.5.0)
*Status: Complete*

- [x] **`path-refactor`**:
    - Global "Search & Destroy" for legacy paths in `.cpp`, `.hpp`, `.sqf`.
    - Context-aware re-prefixing (Identifies PBO root and maps to unit standard).
- [x] **`audit-code-links`**:
    - Extend asset auditor to scan source code for path leaks.
- [x] **Path Guard (HEMTT)**:
    - Pre-build hook to block non-unit path references.

## Phase 7: Strategic Command & Intelligence (v1.6.0)
*Status: Complete*

- [ ] **Asset "Weight" Analytics**:
    - `weight-reporter`: Detect high-poly models and oversized textures.
    - Flag performance bottlenecks before they hit the server.
- [ ] **Dependency Graph Analysis**:
    - `dep-graph`: Map project inter-dependencies and flag circular links.
- [x] **The "Platinum Health Score"**:
    - A centralized metric (0-100) for every project based on DevOps compliance.
- [x] **Automated Workshop Sync**:
    - Dynamic update of Workshop descriptions and "Required Items" via CLI.

---
*Maintained by UKSFTA Senior Production Engineer*

## Phase 9: Tactical Optimization (v1.8.0)
*Status: Initializing*

- [ ] **Unit-Wide Normalization**: Bulk migrate all unit repos to `z\uksftaddons`.
- [ ] **`asset-optimizer`**: Automated texture downscaling and LOD pruning.
- [ ] **`audit-preset`**: Compliance check for external Workshop modlists.

