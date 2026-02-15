#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
import re
from pathlib import Path

# --- CONFIGURATION ---
TOOLS_ROOT = Path(__file__).parent.parent
DEBINARIZER = TOOLS_ROOT / "bin" / "linux-x64" / "debinarizer"
UNIT_PREFIX = r"z\uksfta\addons"

def normalize_vfs(path):
    """Normalizes any path to Arma VFS standard (lowercase, backslashes)."""
    return path.lower().replace("/", "\\").lstrip("\\")

def get_addons_in_project(project_path):
    """Maps addon directory names to their $PBOPREFIX$."""
    addons = {}
    addons_dir = Path(project_path) / "addons"
    if not addons_dir.exists():
        return addons
        
    for item in addons_dir.iterdir():
        if item.is_dir():
            prefix_file = item / "$PBOPREFIX$"
            if prefix_file.exists():
                try:
                    prefix = prefix_file.read_text().strip().lower().replace("/", "\\")
                    addons[prefix] = item
                except: pass
    return addons

def audit_p3d(p3d_path):
    """Extracts VFS links from a P3D file."""
    if not DEBINARIZER.exists():
        return []
    textures = []
    cmd = [str(DEBINARIZER), str(p3d_path), "-info"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode == 0:
            found_vfs = False
            for line in res.stdout.splitlines():
                if "[VFS Links]" in line:
                    found_vfs = True
                    continue
                if found_vfs and line.strip().startswith("-"):
                    tex = line.strip()[1:].strip()
                    if tex: textures.append(tex)
    except: pass
    return textures

def validate_vfs_links(textures, project_addons):
    """Validates VFS links against project addons and standard A3 paths."""
    leaks = []
    missing = []
    
    for t in textures:
        t_low = normalize_vfs(t)
        
        # 1. Skip standard A3 paths
        if t_low.startswith("a3\\"):
            continue
            
        # 2. Check if it belongs to any addon in THIS project
        found_in_project = False
        for prefix, addon_path in project_addons.items():
            if t_low.startswith(prefix):
                found_in_project = True
                # Extract relative path from prefix
                rel_path = t_low[len(prefix):].lstrip("\\")
                # Convert backslashes to system separators for existence check
                sys_rel_path = rel_path.replace("\\", os.sep)
                full_path = addon_path / sys_rel_path
                
                if not full_path.exists():
                    # Fallback: check if the filename exists anywhere in the addon
                    fname = Path(sys_rel_path).name.lower()
                    file_exists = False
                    for root, _, files in os.walk(addon_path):
                        if fname in [f.lower() for f in files]:
                            file_exists = True
                            break
                    if not file_exists:
                        missing.append(t)
                break
        
        # 3. If it starts with unit prefix but not in this project, it's an external unit dependency
        # If it doesn't start with unit prefix, it's a leak.
        if not found_in_project:
            if not t_low.startswith(UNIT_PREFIX.lower()):
                leaks.append(t)
                
    return leaks, missing

def audit_project_assets(project_path):
    project_path = Path(project_path).resolve()
    print(f"\n🛡️  [Assurance Engine] Auditing: {project_path.name}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    
    project_addons = get_addons_in_project(project_path)
    asset_exts = {".paa", ".p3d", ".wav", ".ogg", ".ogv", ".wrp", ".rtm"}
    code_exts = {".cpp", ".hpp", ".sqf", ".xml", ".rvmat"}
    
    all_files = []
    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            all_files.append(Path(root) / f)

    assets = [f for f in all_files if f.suffix.lower() in asset_exts]
    code_files = [f for f in all_files if f.suffix.lower() in code_exts]
    
    if not assets:
        print("  ℹ️  No binary assets found.")
        return

    # 1. REFERENCE AUDIT (ORPHAN DETECTION)
    code_refs = set()
    # Path regex: capture anything that looks like a path or filename with extension
    path_regex = re.compile(r'([a-zA-Z0-9_\\./-]+\.(paa|rvmat|p3d|ogg|wss|rtm|wrp))', re.IGNORECASE)
    
    for c in code_files:
        try:
            content = c.read_text(errors='ignore')
            matches = path_regex.findall(content)
            for m in matches:
                full_ref = m[0].lower().replace("/", "\\")
                code_refs.add(full_ref)
                # Also add just the filename for loose matching
                code_refs.add(full_ref.split("\\")[-1])
        except: pass

    orphans = []
    for a in assets:
        if a.name.lower() not in code_refs:
            orphans.append(a.relative_to(project_path))

    # 2. VFS LINK AUDIT
    all_leaks = []
    all_missing = []
    
    # Scan P3Ds
    for a in assets:
        if a.suffix.lower() == ".p3d":
            textures = audit_p3d(a)
            leaks, missing = validate_vfs_links(textures, project_addons)
            all_leaks.extend([(a.name, l) for l in leaks])
            all_missing.extend([(a.name, m) for m in missing])

    # Scan Source Code
    for c in code_files:
        try:
            content = c.read_text(errors='ignore')
            matches = path_regex.findall(content)
            code_paths = [m[0] for m in matches]
            leaks, missing = validate_vfs_links(code_paths, project_addons)
            all_leaks.extend([(c.name, l) for l in leaks])
            all_missing.extend([(c.name, m) for m in missing])
        except: pass

    # 3. SUMMARY REPORT
    print("\n[Summary Report]")
    
    if orphans:
        print(f"  ❌ {len(orphans)} Orphaned Assets (Unused in code)")
        # Show sample of orphans
        for o in sorted(orphans)[:5]:
            print(f"     - {o}")
        if len(orphans) > 5: print(f"     ... and {len(orphans)-5} more.")
    else:
        print("  ✅ Reference Integrity: PASS")

    if all_leaks:
        unique_leaks = sorted(list(set(all_leaks)))
        print(f"  ⚠️  {len(unique_leaks)} External Leaks Detected (Non-unit paths):")
        for source, leak in unique_leaks[:10]:
            print(f"     - {source} -> {leak}")
    else:
        print("  ✅ External Leakage: NONE")

    if all_missing:
        unique_missing = sorted(list(set(all_missing)))
        print(f"  ❌ {len(unique_missing)} Missing Internal Links (Dead VFS paths):")
        for source, miss in unique_missing[:10]:
            print(f"     - {source} -> {miss}")
    else:
        print("  ✅ VFS Link Integrity: PASS")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: asset_auditor.py <project_path>")
        sys.exit(1)
    audit_project_assets(sys.argv[1])
