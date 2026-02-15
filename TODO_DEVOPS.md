# 🛠 UKSFTA Platinum DevOps Suite - Roadmap

## 🎯 Global Standards

- [ ] **100% Cross-Compatibility:** All tools must run natively on Linux-x64 and Windows-x64.
- [ ] **Path Normalization:** Automate `` vs `/` conversion for Arma VFS vs System FS.
- [ ] **Atomic Operations:** Ensure file writes are atomic to prevent asset corruption during build failures.
- [ ] **GPG Signed:** All tool updates must be cryptographically signed.

---

## Phase 1: Assurance Engine (v1.3.0)

*Status: Complete*

- [x] **`audit-lods`**: Automated detection of missing Shadow Volume and Geometry LODs.
- [x] **`audit-vfs-links`**: Deep-scan P3D and source code for path leaks.
- [ ] **`optimize-dashboard`**: Visual CLI report of project "weight".

## Phase 2: Migration & Refactor Suite

*Status: Complete*

- [x] **`remap-advanced`**: Bone/Material remapping and path renaming.
- [x] **`manage-proxies`**: CLI-based proxy listing and sanitization framework.
- [x] **`rebin-guard`**: Pre-binarization quality assurance.

## Phase 3: Forensic Intelligence

*Status: Complete*

- [x] **`diff-models`**: Binary-level comparison of P3D assets.
- [x] **`auto-classifier`**: Automated asset categorization via internal metadata.

## Phase 4: HEMTT Deep Integration

*Status: Complete*

- [x] **`pre_build` Forensic Hook**: Automated asset validation during build cycle.
- [ ] **Unit Signer Integration**: Seamless signing within the pipeline.

## Phase 5: Asset Ingestion & Porting (v1.4.0)

*Status: Complete*

- [x] **`import-wizard`**: One-click ingestion of external assets.

## Phase 6: Global Path Normalization (v1.5.0)

*Status: Complete*

- [x] **`path-refactor`**: Recursive automated path repair in code and assets.

## Phase 7: Strategic Command & Intelligence (v1.6.0)

*Status: Complete*

- [x] **`weight-reporter`**: Performance analytics for high-poly/large-texture assets.
- [x] **`dep-graph`**: Unit-wide dependency mapping.
- [x] **`trend-analyze`**: Longitudinal health tracking and dashboard.

## Phase 10: Advanced Operations (v1.9.0)

*Status: Complete*

- [x] **`audit-preset`**: Compliance check for external Workshop modlists.
- [ ] **Unit Signer**: Automated .bisign generation.
- [ ] **Remote-Orchestrator**: One-click build delegation to VPS nodes.

## Phase 11: Deep Inspection (v2.0.0)

*Status: Initializing*

- [x] **`pbo-cracker`**:
  - Integrate `hemtt extract` for just-in-time forensic auditing of packed assets.
  - Automated recursive unpacking of large modsets.
- [ ] **Active Asset Optimization**:
  - Functional `asset-optimizer --apply` with PAA -> PNG -> PAA bridge.
- [ ] **The Virtual Armory**:
  - Automated visual asset catalog generation.

---
*Maintained by UKSFTA Senior Production Engineer*
