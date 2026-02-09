// Verify all addon folders have a $PBOPREFIX$ file
// This prevents broken references in complex mod structures

let addons = HEMTT_RFS.join("addons").list();
let missing = 0;

for addon in addons {
    if addon.is_dir() {
        let prefix_file = addon.join("$PBOPREFIX$");
        if !prefix_file.exists() {
            print("ERROR: Missing $PBOPREFIX$ in " + addon.name());
            missing += 1;
        }
    }
}

if missing > 0 {
    print("Found " + missing.to_string() + " addons missing $PBOPREFIX$");
} else {
    print("All addons have $PBOPREFIX$ files.");
}
