#!/usr/bin/env python3
import json
import os
import re
import argparse
from pathlib import Path

# UKSFTA Global Manifest Generator
# Creates a machine-readable JSON structure of the entire unit workspace.

def generate_total_manifest(root_dir, dry_run=False):
    root = Path(root_dir)
    projects = []
    
    for d in sorted(root.iterdir()):
        if d.is_dir() and d.name.startswith("UKSFTA-") and (d / ".hemtt" / "project.toml").exists():
            proj_info = {
                "name": d.name,
                "version": "0.0.0",
                "components": [],
                "external_mods": []
            }
            
            # Version
            v_file = d / "addons" / "main" / "script_version.hpp"
            if v_file.exists():
                vc = v_file.read_text()
                ma = re.search(r'#define MAJOR (.*)', vc)
                mi = re.search(r'#define MINOR (.*)', vc)
                pa = re.search(r'#define PATCHLVL (.*)', vc)
                if ma and mi and pa:
                    proj_info["version"] = f"{ma.group(1).strip()}.{mi.group(1).strip()}.{pa.group(1).strip()}"

            # Components
            addons = d / "addons"
            if addons.exists():
                proj_info["components"] = sorted([entry.name for entry in addons.iterdir() if entry.is_dir() and not entry.name.startswith(".")])

            # External Deps
            src = d / "mod_sources.txt"
            if src.exists():
                with open(src, 'r') as f:
                    for line in f:
                        if "[ignore]" in line.lower(): break
                        m = re.search(r"(\d{8,})", line)
                        if m:
                            name = line.split("#", 1)[1].strip() if "#" in line else f"Mod {m.group(1)}"
                            proj_info["external_mods"].append({"id": m.group(1), "name": name})
            
            projects.append(proj_info)

    manifest = {
        "unit": "UKSF Taskforce Alpha",
        "generated": Path(__file__).stat().st_mtime,
        "projects": projects
    }

    if dry_run:
        print("\n--- [DRY-RUN] Global Manifest Preview ---")
        print(json.dumps(manifest, indent=2))
        print("-----------------------------------------\n")
    else:
        (root / "unit_manifest.json").write_text(json.dumps(manifest, indent=4))
        print(f"✅ Manifest generated: {root / 'unit_manifest.json'}")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Manifest Generator")
    parser.add_argument("--dry-run", action="store_true", help="Preview manifest in console")
    args = parser.parse_args()
    
    generate_total_manifest(Path(__file__).parent.parent, args.dry_run)

if __name__ == "__main__":
    main()
