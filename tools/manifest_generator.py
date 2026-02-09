#!/usr/bin/env python3
import os
import sys
import re
from pathlib import Path

def generate_total_manifest(workspace_path):
    print(f"📋 Generating Global Unit Manifest...")
    
    # Use the parent directory of Tools to find other repos
    parent_dir = Path(workspace_path).parent
    projects = [d for d in parent_dir.iterdir() if d.is_dir() and (d / ".hemtt" / "project.toml").exists()]
    
    external_mods = {} # ID -> Name/Tag
    internal_pbos = {} # Project -> List of PBOs
    
    for p in sorted(projects):
        # 1. Parse mod_sources.txt
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
                        name = clean.split("#", 1)[1].strip() if "#" in clean else "Unknown Mod"
                        external_mods[mid] = name

        # 2. Scan for local PBOs
        addons_dir = p / "addons"
        if addons_dir.exists():
            pbos = []
            for entry in addons_dir.iterdir():
                if entry.is_dir() and not entry.name.startswith("."):
                    pbos.append(entry.name)
                elif entry.suffix.lower() == ".pbo":
                    pbos.append(entry.stem)
            if pbos:
                internal_pbos[p.name] = sorted(list(set(pbos)))

    # 3. Build the Report
    report = []
    report.append("============================================================")
    report.append("          UKSF TASKFORCE ALPHA - TOTAL MANIFEST             ")
    report.append(f"          Generated: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    report.append("============================================================
")

    report.append("--- INTERNAL UNIT COMPONENTS (PBOs) ---")
    for proj, pbos in internal_pbos.items():
        report.append(f"[{proj}]")
        for pbo in pbos:
            report.append(f"  - {pbo}")
    report.append("")

    report.append("--- EXTERNAL WORKSHOP DEPENDENCIES ---")
    for mid in sorted(external_mods.keys()):
        report.append(f"  ID: {mid:<12} | {external_mods[mid]}")
    
    report.append("
============================================================")
    report.append(f"  Summary: {len(external_mods)} External Mods | {sum(len(v) for v in internal_pbos.values())} Internal PBOs")
    report.append("============================================================")

    # 4. Save to central hub
    output_path = Path(workspace_path) / "all_releases" / "TOTAL_MANIFEST.txt"
    os.makedirs(output_path.parent, exist_ok=True)
    output_path.write_text("
".join(report))
    
    print(f"  ✅ Manifest generated: {output_path}")
    return output_path

if __name__ == "__main__":
    from datetime import datetime
    generate_total_manifest(os.getcwd())
