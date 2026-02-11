// UKSFTA HEMTT Hook: CfgPatches Sync
// Scans CfgVehicles, CfgWeapons, and CfgUnitInsignia to populate units[] and weapons[]

print("HOOK: Synchronizing CfgPatches metadata...");

fn extract_classes(content, section) {
    let classes = [];
    let start_idx = content.index_of("class " + section);
    if start_idx < 0 { return classes; }
    
    // Find the end of this specific section block
    // We search for the next top-level class or end of file
    let end_idx = content.index_of("class Cfg", start_idx + 10);
    if end_idx < 0 { end_idx = content.len(); }
    
    let block = content.sub_string(start_idx, end_idx - start_idx);
    let lines = block.split("\n");
    
    for line in lines {
        if line.contains("class") && line.contains("{") {
            let c_idx = line.index_of("class");
            let colon_idx = line.index_of(":");
            let open_idx = line.index_of("{");
            
            // Determine if it's an inheritance or direct class
            let end_name_idx = -1;
            if colon_idx > c_idx && colon_idx < open_idx {
                end_name_idx = colon_idx;
            } else {
                end_name_idx = open_idx;
            }
            
            if end_name_idx > c_idx {
                let name = line.sub_string(c_idx + 5, end_name_idx - c_idx - 5);
                
                // Clean name
                let clean_name = "";
                let name_raw = name.split(" ");
                for n in name_raw { if n.len() > 0 { clean_name = n; } }
                
                // Filter out base/system classes
                if clean_name.len() > 0 && !clean_name.contains("Cfg") && clean_name != section {
                    classes.push(clean_name);
                }
            }
        }
    }
    return classes;
}

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
                    let units = extract_classes(content, "CfgVehicles");
                    let insignia = extract_classes(content, "CfgUnitInsignia");
                    let weapons = extract_classes(content, "CfgWeapons");

                    if (units.len() > 0 || weapons.len() > 0 || insignia.len() > 0) {
                        print("  - " + name + ": Found " + units.len() + " units, " + insignia.len() + " insignia, " + weapons.len() + " weapons");
                        
                        // All found classes for units[]
                        let all_units = [];
                        for u in units { all_units.push(u); }
                        for i in insignia { all_units.push(i); }

                        let units_str = "units[] = {";
                        let u_first = true;
                        for u in all_units {
                            if (!u_first) { units_str = units_str + ","; }
                            units_str = units_str + "\"" + u + "\"";
                            u_first = false;
                        }
                        units_str = units_str + "};";

                        let weapons_str = "weapons[] = {";
                        let w_first = true;
                        for w in weapons {
                            if (!w_first) { weapons_str = weapons_str + ","; }
                            weapons_str = weapons_str + "\"" + w + "\"";
                            w_first = false;
                        }
                        weapons_str = weapons_str + "};";

                        // Update metadata in config.cpp
                        let u_idx = content.index_of("units[]");
                        if (u_idx >= 0) {
                            let u_end_idx = content.index_of("};", u_idx);
                            let u_old_block = content.sub_string(u_idx, u_end_idx - u_idx + 2);
                            content.replace(u_old_block, units_str);
                        }

                        let w_idx = content.index_of("weapons[]");
                        if (w_idx >= 0) {
                            let w_end_idx = content.index_of("};", w_idx);
                            let w_old_block = content.sub_string(w_idx, w_end_idx - w_idx + 2);
                            content.replace(w_old_block, weapons_str);
                        }
                        
                        config_path.create_file().write(content);
                        print("    [!] Metadata synchronized.");
                    }
                }
            }
        }
    }
}
print("HOOK: Sync complete.");
