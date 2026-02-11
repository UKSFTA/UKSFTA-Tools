// UKSFTA HEMTT Hook: CfgPatches Sync
// Scans CfgVehicles and CfgWeapons to automatically populate units[] and weapons[]

fn process_addon(name) {
    let config_path = HEMTT_RFS.join("addons").join(name).join("config.cpp");
    if !config_path.exists() { return; }

    let content = config_path.open_file().read();
    
    // 1. Find all classes in CfgVehicles with scope=2
    let units = [];
    let vehicles_match = content.regex_match("class\\s+CfgVehicles\\s*\\{[^}]*\\}", "m");
    if vehicles_match.len() > 0 {
        let vehicles_block = vehicles_match[0];
        let classes = vehicles_block.regex_matches("class\\s+([\\w_]+)(?:\\s*:\\s*[\\w_]+)?\\s*\\{[^}]*scope\\s*=\\s*2", "m");
        for c in classes {
            units.push(c[1]);
        }
    }

    // 2. Find all classes in CfgWeapons with scope=2
    let weapons = [];
    let weapons_match = content.regex_match("class\\s+CfgWeapons\\s*\\{[^}]*\\}", "m");
    if weapons_match.len() > 0 {
        let weapons_block = weapons_match[0];
        let classes = weapons_block.regex_matches("class\\s+([\\w_]+)(?:\\s*:\\s*[\\w_]+)?\\s*\\{[^}]*scope\\s*=\\s*2", "m");
        for c in classes {
            weapons.push(c[1]);
        }
    }

    if units.len() > 0 || weapons.len() > 0 {
        print("  - " + name + ": Syncing " + units.len() + " units and " + weapons.len() + " weapons");
        
        // Format the arrays for CfgPatches
        let units_str = "units[] = {" + units.map(|u| "\"" + u + "\"").join(",") + "};";
        let weapons_str = "weapons[] = {" + weapons.map(|w| "\"" + w + "\"").join(",") + "};";

        // Update the file content
        content = content.regex_replace("units\\[\\]\\s*=\\s*\\{[^}]*\\};", units_str);
        content = content.regex_replace("weapons\\[\\]\\s*=\\s*\\{[^}]*\\};", weapons_str);
        
        config_path.create_file().write(content);
    }
}

print("HOOK: Synchronizing CfgPatches metadata...");
let addons_dir = HEMTT_RFS.join("addons");
if addons_dir.exists() {
    for addon in addons_dir.list() {
        if addon.is_dir() {
            // In Rhai PathBuf, to_string() gives full path, we need the leaf name
            let full_path = addon.to_string();
            let name = full_path.regex_match("[^\\\\/]+$", "m")[0];
            process_addon(name);
        }
    }
}
