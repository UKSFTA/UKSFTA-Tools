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

- [ ] **`pre_build` Forensic Hook**:
    - Execute JIT (Just-In-Time) debinarization for third-party submodules.
    - Dynamic path re-prefixing during the HEMTT build cycle.
- [ ] **Unit Signer Integration**:
    - Seamless signing of debinarized artifacts within the HEMTT pipeline.

---
*Maintained by UKSFTA Senior Production Engineer*
