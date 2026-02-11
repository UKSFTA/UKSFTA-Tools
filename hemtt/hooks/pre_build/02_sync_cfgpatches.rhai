// UKSFTA HEMTT Hook: CfgPatches Sync
// Scans CfgVehicles and CfgWeapons to automatically populate units[] and weapons[]

fn process_addon(addon) {
    let config_path = addon.path() + "/config.cpp";
    if !HEMTT.exists(config_path) { return; }

    let content = HEMTT.read_file(config_path);
    
    // 1. Find all classes in CfgVehicles with scope=2
    // We look for both new classes and overwrites
    let units = [];
    let vehicles_match = content.regex_match("class\\s+CfgVehicles\\s*\\{[^}]*\\}", "m");
    if vehicles_match.len() > 0 {
        let vehicles_block = vehicles_match[0];
        // Regex to find class names that have scope = 2 inside their block
        // This is a simplified parser for Rhai
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
        print("  - " + addon.name() + ": Syncing " + units.len() + " units and " + weapons.len() + " weapons");
        
        // Format the arrays for CfgPatches
        let units_str = "units[] = {" + units.map(|u| "\"" + u + "\"").join(",") + "};";
        let weapons_str = "weapons[] = {" + weapons.map(|w| "\"" + w + "\"").join(",") + "};";

        // Update the file content
        content = content.regex_replace("units\\[\\]\\s*=\\s*\\{[^}]*\\};", units_str);
        content = content.regex_replace("weapons\\[\\]\\s*=\\s*\\{[^}]*\\};", weapons_str);
        
        HEMTT.write_file(config_path, content);
    }
}

print("HOOK: Synchronizing CfgPatches metadata...");
for addon in HEMTT.project().addons() {
    process_addon(addon);
}
