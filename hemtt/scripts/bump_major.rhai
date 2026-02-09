// Read the current contents of script_version.hpp
let script_version = HEMTT_RFS.join("addons")
    .join("main")
    .join("script_version.hpp")
    .open_file()
    .read();

// Replace the current version with the new version
let prefix = "#define MAJOR ";
let current = HEMTT.project().version().major();
let next = current + 1;

script_version.replace(prefix + current.to_string(), prefix + next.to_string());

// Reset minor and patch
script_version.replace("#define MINOR " + HEMTT.project().version().minor().to_string(), "#define MINOR 0");
script_version.replace("#define PATCHLVL " + HEMTT.project().version().patch().to_string(), "#define PATCHLVL 0");

// Write the modified contents to script_version.hpp
HEMTT_RFS.join("addons")
    .join("main")
    .join("script_version.hpp")
    .create_file()
    .write(script_version);
print("Major version bumped to " + next.to_string());
