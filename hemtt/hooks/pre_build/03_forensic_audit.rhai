// UKSFTA Forensic Audit Hook
// Executes Phase 1 & 2 safety checks during the HEMTT build cycle.

let project = hemtt.project();
print("🛡️  [HEMTT] Starting UKSFTA Forensic Audit...");

// We use the Python orchestrator to run the rebin-guard on all P3Ds
// This ensures that any missing Geometry LODs or path leaks fail the build.

let result = system.run("python3", ["tools/asset_auditor.py", "."]);

if result != 0 {
    fatal("❌ Asset Forensic Audit FAILED. Building halted to prevent artifact corruption.");
}

print("✅ [HEMTT] Forensic Audit Passed.");
