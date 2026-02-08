print("HEMTT: Normalizing output timestamps...");

// Use our centralized python tool which handles OS differences
HEMTT.exec("python3", ["tools/fix_timestamps.py", ".hemttout"]);

print("HEMTT: Timestamps normalized.");
