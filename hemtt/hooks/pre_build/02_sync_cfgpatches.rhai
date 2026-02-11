// UKSFTA HEMTT Hook: CfgPatches Sync
// Scans CfgVehicles and CfgWeapons to populate units[] and weapons[]

print("HOOK: Synchronizing CfgPatches metadata...");

fn extract_top_level_classes(content, section) {
    let classes = [];
    let section_key = "class " + section;
    let start_idx = content.index_of(section_key);
    if start_idx < 0 { return classes; }
    
    let next_cfg = content.index_of("class Cfg", start_idx + section_key.len());
    let end_idx = if next_cfg > 0 { next_cfg } else { content.len() };
    
    let block = content.sub_string(start_idx, end_idx - start_idx);
    let lines = block.split("\n");
    
    // Internal Arma classes that should NEVER be synced to units[]
    let internal_classes = [
        "HitPoints", "HitEngine", "HitFuel", "Turrets", "AnimationSources", 
        "UserActions", "EventHandlers", "CargoTurret", "MainTurret", "NewTurret",
        "ViewPilot", "ViewOptics", "Reflectors", "Exhausts", "Damage", "SimpleObject",
        "AcreRacks", "complexGearbox", "Wheels"
    ];

    for i in 0..lines.len() {
        let line = lines[i];
        if line.contains("class") && line.contains("{") && !line.contains(section) {
            let c_idx = line.index_of("class");
            let colon_idx = line.index_of(":");
            let open_idx = line.index_of("{");
            
            let name_end = if colon_idx > c_idx && colon_idx < open_idx { colon_idx } else { open_idx };
            
            if name_end > c_idx {
                let name = line.sub_string(c_idx + 5, name_end - c_idx - 5);
                
                // Manual trim
                let clean_name = "";
                let name_parts = name.split(" ");
                for p in name_parts { if p.len() > 0 { clean_name = p; } }
                
                if clean_name.len() > 0 && !clean_name.contains("Cfg") {
                    let is_internal = false;
                    for internal in internal_classes {
                        if clean_name == internal { is_internal = true; }
                    }
                    
                    if !is_internal {
                        // SCOPED CHECK: find the end of THIS class block
                        let has_display_name = false;
                        let look_ahead = 0;
                        
                        // We search for displayName until we hit the class closure };
                        for j in i..lines.len() {
                            let l = lines[j];
                            if l.contains("displayName") {
                                has_display_name = true;
                                break;
                            }
                            if l.contains("};") && j > i {
                                break;
                            }
                            // Safety break to avoid infinite scan
                            if (j - i) > 50 { break; }
                        }

                        if has_display_name {
                            classes.push(clean_name);
                        }
                    }
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
                    let units = extract_top_level_classes(content, "CfgVehicles");
                    let weapons = extract_top_level_classes(content, "CfgWeapons");

                    if (units.len() > 0 || weapons.len() > 0) {
                        print("  - " + name + ": Syncing " + units.len() + " units and " + weapons.len() + " weapons");
                        
                        let units_str = "units[] = {";
                        let first_u = true;
                        for u in units {
                            if !first_u { units_str = units_str + ","; }
                            units_str = units_str + "\"" + u + "\"";
                            first_u = false;
                        }
                        units_str = units_str + "};";

                        let weapons_str = "weapons[] = {";
                        let first_w = true;
                        for w in weapons {
                            if !first_w { weapons_str = weapons_str + ","; }
                            weapons_str = weapons_str + "\"" + w + "\"";
                            first_w = false;
                        }
                        weapons_str = weapons_str + "};";

                        let u_idx = content.index_of("units[]");
                        if (u_idx >= 0) {
                            let end_u = content.index_of("};", u_idx);
                            let old_u = content.sub_string(u_idx, end_u - u_idx + 2);
                            content.replace(old_u, units_str);
                        }

                        let w_idx = content.index_of("weapons[]");
                        if (w_idx >= 0) {
                            let end_w = content.index_of("};", w_idx);
                            let old_w = content.sub_string(w_idx, end_w - w_idx + 2);
                            content.replace(old_w, weapons_str);
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
