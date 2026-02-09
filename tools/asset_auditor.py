#!/usr/bin/env python3
import os
import sys
from pathlib import Path

def audit_project_assets(project_path):
    print(f"🔍 Auditing Assets for: {os.path.basename(project_path)}")
    
    asset_exts = {".paa", ".p3d", ".wav", ".ogg", ".ogv", ".wrp", ".rtm"}
    code_exts = {".cpp", ".hpp", ".sqf", ".xml"}
    
    all_files = []
    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            all_files.append(Path(root) / f)

    assets = [f for f in all_files if f.suffix.lower() in asset_exts]
    code_files = [f for f in all_files if f.suffix.lower() in code_exts]
    
    if not assets:
        print("  No binary assets found.")
        return

    # Read all code into memory for fast searching
    big_code_blob = ""
    for c in code_files:
        try:
            big_code_blob += c.read_text(errors='ignore').lower()
        except: pass

    orphans = []
    for a in assets:
        # Check if filename (case insensitive) is in code
        name = a.name.lower()
        if name not in big_code_blob:
            # Also check without extension (sometimes used in config)
            if a.stem.lower() not in big_code_blob:
                orphans.append(a.relative_to(project_path))

    if orphans:
        print(f"  ❌ Found {len(orphans)} orphaned assets (not referenced in code):")
        for o in sorted(orphans):
            print(f"     - {o}")
    else:
        print("  ✅ All assets are correctly referenced.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: audit_assets.py <project_path>")
        sys.exit(1)
    audit_project_assets(sys.argv[1])
