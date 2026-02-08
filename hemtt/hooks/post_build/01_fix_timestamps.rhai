print("HEMTT: Normalizing output timestamps...");

if HEMTT.is_windows() {
    // Windows: Use PowerShell to touch all files
    HEMTT.exec("powershell", ["-Command", "Get-ChildItem -Path .hemttout -Recurse | ForEach-Object { $_.LastWriteTime = Get-Date }"]);
} else {
    // Linux: Use find + touch
    HEMTT.exec("find", [".hemttout", "-exec", "touch", "{}", "+"]);
}

print("HEMTT: Timestamps normalized.");
