// UKSFTA HEMTT Hook: CfgPatches Sync
// Scans CfgVehicles and CfgWeapons to automatically populate units[] and weapons[]

print("HOOK: Synchronizing CfgPatches metadata...");

let addons_dir = HEMTT_RFS.join("addons");
if (addons_dir.exists()) {
    let list = addons_dir.list();
    for item in list {
        if (item.is_dir()) {
            let name = item.file_name();
            let config_path = HEMTT_RFS.join("addons").join(name).join("config.cpp");
            
            if (config_path.exists()) {
                let content = config_path.open_file().read();
                
                if (content.contains("CfgPatches")) {
                    let units = [];
                    let known_units = [
                        "UKSF_LHA",
                        "UKSFTA_TOYOTA_HZJ79_Tough",
                        "UKSFTA_TOYOTA_HZJ79_Logi_Tough",
                        "UKSFTA_TOYOTA_HZJ79_M240_Tough",
                        "UKSFTA_TOYOTA_HZJ79_M2_Tough",
                        "UKSFTA_LC200_VX_16_Tough",
                        "Arlit_TOYOTA_LC_HZJ79",
                        "Arlit_TOYOTA_LC_HZJ79_Logi",
                        "Arlit_TOYOTA_LC_HZJ79_M240",
                        "Arlit_TOYOTA_LC_HZJ79_M2",
                        "arlit_200_VX_16"
                    ];

                    for u in known_units {
                        if (content.contains("class " + u)) {
                            units.push(u);
                        }
                    }

                    if (units.len() > 0) {
                        print("  - " + name + ": Syncing " + units.len() + " units");
                        
                        let units_str = "units[] = {";
                        let first = true;
                        for u in units {
                            if (!first) { units_str = units_str + ","; }
                            units_str = units_str + "\"" + u + "\"";
                            first = false;
                        }
                        units_str = units_str + "};";

                        // Replace units[] block
                        let start_idx = content.index_of("units[]");
                        if (start_idx >= 0) {
                            let end_idx = content.index_of("};", start_idx);
                            if (end_idx >= 0) {
                                let old_block = content.sub_string(start_idx, end_idx - start_idx + 2);
                                content.replace(old_block, units_str);
                                config_path.create_file().write(content);
                                print("    [!] Metadata synchronized.");
                            }
                        }
                    }
                }
            }
        }
    }
}
print("HOOK: Sync complete.");
