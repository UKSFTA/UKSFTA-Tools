#!/usr/bin/env python3
import os
import sys
import re
from pathlib import Path
from datetime import datetime

def generate_total_manifest(workspace_path):
    print(f"📋 Generating Global Unit Manifest...")
    
    parent_dir = Path(workspace_path).parent
    projects = [d for d in parent_dir.iterdir() if d.is_dir() and (d / ".hemtt" / "project.toml").exists()]
    
    external_mods = {} # ID -> {"name": str, "sources": list}
    internal_mods = [] 
    
    for p in sorted(projects):
        sources_path = p / "mod_sources.txt"
        if sources_path.exists():
            with open(sources_path, 'r') as f:
                for line in f:
                    clean = line.strip()
                    if not clean or clean.startswith("#"): continue
                    if "[ignore]" in clean.lower(): break
                    
                    match = re.search(r"(?:id=)?(\d{8,})", clean)
                    if match:
                        mid = match.group(1)
                        name = clean.split("#", 1)[1].strip() if "#" in clean else f"Workshop Mod {mid}"
                        
                        if mid not in external_mods:
                            external_mods[mid] = {"name": name, "sources": []}
                        if p.name not in external_mods[mid]["sources"]:
                            external_mods[mid]["sources"].append(p.name)

        internal_mods.append(p.name)

    report = []
    report.append("============================================================")
    report.append("          UKSF TASKFORCE ALPHA - TOTAL MANIFEST             ")
    report.append(f"          Generated: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    report.append("============================================================\n")

    report.append("--- INTERNAL UNIT MODS ---")
    for mod in sorted(internal_mods):
        report.append(f"  [x] {mod}")
    report.append("")

    report.append("--- EXTERNAL WORKSHOP DEPENDENCIES ---")
    # Sort external mods by name instead of ID
    sorted_mids = sorted(external_mods.keys(), key=lambda x: external_mods[x]["name"].lower())
    
    for mid in sorted_mids:
        data = external_mods[mid]
        sources_str = ", ".join(sorted(data["sources"]))
        report.append(f"  Mod:    {data['name']}")
        report.append(f"  Link:   https://steamcommunity.com/sharedfiles/filedetails/?id={mid}")
        report.append(f"  ID:     {mid}")
        report.append(f"  Origin: Required by [{sources_str}]")
        report.append("-" * 30)
    
    report.append("\n============================================================")
    report.append(f"  Summary: {len(external_mods)} External Dependencies | {len(internal_mods)} Internal Modules")
    report.append("============================================================")

    output_path = Path(workspace_path) / "all_releases" / "TOTAL_MANIFEST.txt"
    os.makedirs(output_path.parent, exist_ok=True)
    output_path.write_text("\n".join(report))
    
    print(f"  ✅ Manifest generated: {output_path}")
    return output_path

if __name__ == "__main__":
    generate_total_manifest(os.getcwd())
