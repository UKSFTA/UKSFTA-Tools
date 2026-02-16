// UKSFTA Forensic Audit Hook
// Performs native structural and path-integrity checks within HEMTT.

print("🛡️ [HEMTT] Starting UKSFTA Forensic Audit...");

let project_name = HEMTT.project().name();
let clean_project_name = project_name.to_lower();
// Remove UKSFTA- prefix if present
if clean_project_name.index_of("uksfta-") == 0 {
    clean_project_name = clean_project_name.sub_string(7);
}

let addons_dir = HEMTT_RFS.join("addons");
let error_count = 0;

if (addons_dir.exists()) {
    let list = addons_dir.list();
    for item in list {
        if (item.is_dir()) {
            let addon_name = item.file_name();
            
            // --- CHECK 1: Mandatory $PBOPREFIX$ ---
            let prefix_path = item.join("$PBOPREFIX$");
            if (!prefix_path.exists()) {
                print("  ❌ [FORENSIC] " + addon_name + ": Missing $PBOPREFIX$ file!");
                error_count = error_count + 1;
            } else {
                let prefix_content = prefix_path.open_file().read();
                if (prefix_content.len() > 0) {
                    let low_prefix = prefix_content.to_lower();
                    if (low_prefix.index_of("z\\uksfta\\addons") < 0) {
                        print("  ⚠️ [FORENSIC] " + addon_name + ": $PBOPREFIX$ does not follow unit standard (z\\uksfta\\addons\\...)");
                    }
                } else {
                    print("  ❌ [FORENSIC] " + addon_name + ": $PBOPREFIX$ is empty!");
                    error_count = error_count + 1;
                }
            }

            // --- CHECK 2: Forbidden Paths (P:\ Drive leakage) ---
            let config_path = item.join("config.cpp");
            if (config_path.exists()) {
                let content = config_path.open_file().read();
                if (content.index_of("P:\\") >= 0 || content.index_of("p:\\") >= 0) {
                    print("  ❌ [FORENSIC] " + addon_name + ": Local path 'P:\\' detected in config.cpp!");
                    error_count = error_count + 1;
                }
            }
        }
    }
}

if (error_count > 0) {
    print("❌ [FORENSIC] Audit FAILED with " + error_count + " critical errors.");
    HEMTT.fatal("Forensic Audit failed. Check the logs above for details.");
} else {
    print("✅ [FORENSIC] Native Audit Passed.");
}
