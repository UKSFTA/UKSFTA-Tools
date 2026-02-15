# 🛠 UKSFTA Platinum DevOps Suite - Roadmap

## 🎯 Global Standards

- [ ] **100% Cross-Compatibility:** All tools must run natively on Linux-x64 and Windows-x64.
- [ ] **Path Normalization:** Automate `` vs `/` conversion for Arma VFS vs System FS.
- [ ] **Atomic Operations:** Ensure file writes are atomic to prevent asset corruption during build failures.
- [ ] **GPG Signed:** All tool updates must be cryptographically signed.

---

## 🏁 Phase Status: Complete

### Phase 1: Assurance Engine (v1.3.0)

- [x] **`audit-lods`**: Automated detection of missing Shadow Volume and Geometry LODs.
- [x] **`audit-vfs-links`**: Deep-scan P3D and source code for path leaks.
- [ ] **`optimize-dashboard`**: Visual CLI report of project "weight".

### Phase 2: Migration & Refactor Suite

- [x] **`remap-advanced`**: Bone/Material remapping and path renaming.
- [x] **`manage-proxies`**: CLI-based proxy listing and sanitization framework.
- [x] **`rebin-guard`**: Pre-binarization quality assurance.

### Phase 3: Forensic Intelligence

- [x] **`diff-models`**: Binary-level comparison of P3D assets.
- [x] **`auto-classifier`**: Automated asset categorization via internal metadata.

### Phase 4: HEMTT Deep Integration

- [x] **`pre_build` Forensic Hook**: Automated asset validation during build cycle.
- [ ] **Unit Signer Integration**: Seamless signing within the pipeline.

### Phase 5: Asset Ingestion & Porting (v1.4.0)

- [x] **`import-wizard`**: One-click ingestion of external assets.

### Phase 6: Global Path Normalization (v1.5.0)

- [x] **`path-refactor`**: Recursive automated path repair in code and assets.

### Phase 7: Strategic Command & Intelligence (v1.6.0)

- [x] **`weight-reporter`**: Performance analytics for high-poly/large-texture assets.
- [x] **`dep-graph`**: Unit-wide dependency mapping.
- [x] **`trend-analyze`**: Longitudinal health tracking and dashboard.

### Phase 10: Advanced Operations (v1.9.0)

- [x] **`audit-preset`**: Compliance check for external Workshop modlists.
- [ ] **Unit Signer**: Automated .bisign generation.
- [ ] **Remote-Orchestrator**: One-click build delegation to VPS nodes.

### Phase 11: Deep Inspection (v2.0.0)

- [x] **`pbo-cracker`**: Just-in-time forensic auditing of packed mod assets.
- [x] **Active Asset Optimization**: Automated PAA -> PNG -> PAA texture downscaling.

### Phase 12: Extended Integration (v2.1.0)

- [x] **ACE Extended Arsenal Helper**: Automated grouping for ACE Extended Arsenal.
- [x] **Virtual Armory**: Automated visual asset catalog generation.

---

## 🚧 Phase Status: In Progress

### Phase 13: Autonomous Operations (v3.0.0)

- [x] **The Sentinel**: Real-time filesystem watchdog for auto-optimization and validation.
- [x] **The Maintainer**: Self-healing update loop (Sync -> Audit -> Commit/Revert).
- [ ] **The Architect**: Context-aware configuration generation based on asset analysis.

---

#### Maintained by UKSFTA Senior Production Engineer
