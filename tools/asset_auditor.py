#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
import re
import json
from pathlib import Path

# --- CONFIGURATION ---
TOOLS_ROOT = Path(__file__).parent.parent
DEBINARIZER = TOOLS_ROOT / "bin" / "linux-x64" / "debinarizer"
UNIT_PREFIX = r"z\uksfta\addons"

def get_project_vfs_path(project_path):
    r"""Derives the VFS path for a project, e.g. z\uksfta\addons\maps"""
    name = Path(project_path).name.lower()
    if name.startswith("uksfta-"):
        name = name.replace("uksfta-", "")
    return UNIT_PREFIX + "\\" + name

def audit_p3d(p3d_path, project_path):
    """Deep audit of a P3D file using the forensic binary."""
    if not DEBINARIZER.exists():
        return [], []

    results = []
    textures = []
    
    # 1. Forensic Audit (LODs, Integrity)
    cmd = [str(DEBINARIZER), str(p3d_path), "-audit-lods"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True)
        if res.returncode == 0:
            results.append(res.stdout.strip())
        else:
            results.append(f"  [!] Forensic fail: {p3d_path.name} ({res.stderr.strip()})")
    except Exception as e:
        results.append(f"  [!] Failed to execute forensic tool: {e}")

    # 2. Extract VFS Links for Validation
    cmd_info = [str(DEBINARIZER), str(p3d_path), "-info"]
    try:
        res = subprocess.run(cmd_info, capture_output=True, text=True)
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
    
    return results, textures

def validate_vfs_links(textures, project_path):
    """Validates that VFS links exist or are internal to the unit."""
    leaks = []
    missing = []
    
    project_vfs = get_project_vfs_path(project_path).lower()
    
    for t in textures:
        # Normalize to backslash for comparison
        t_low = t.lower().replace("/", "\\")
        
        # 1. Check for External Leakage
        # If it doesn't start with unit prefix, it's a leak
        if not t_low.startswith(UNIT_PREFIX.lower()) and not t_low.startswith("\\" + UNIT_PREFIX.lower()):
            # Ignore standard A3 paths
            if not any(x in t_low for x in ["a3\\", "\\a3\\"]):
                leaks.append(t)
                continue

        # 2. Check for Missing Internal Links
        if project_vfs in t_low:
            # Strip VFS prefix to get relative path
            rel_path = t_low.split(project_vfs)[-1].lstrip("\\")
            # Replace VFS backslashes with system separators
            rel_sys_path = rel_path.replace("\\", os.sep)
            
            full_path = Path(project_path) / "addons" / Path(project_path).name.replace("UKSFTA-", "").lower() / rel_sys_path
            
            if not os.path.exists(full_path):
                # Fallback search in entire project
                found = False
                for root, _, files in os.walk(project_path):
                    if Path(t).name.lower() in [f.lower() for f in files]:
                        found = True
                        break
                if not found: missing.append(t)

    return leaks, missing

def audit_project_assets(project_path):
    project_path = Path(project_path)
    print(f"\n🛡️  [Assurance Engine] Auditing: {project_path.name}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    
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
        print("  ℹ️  No binary assets found.")
        return

    # 1. ORPHAN AUDIT
    big_code_blob = ""
    for c in code_files:
        try:
            big_code_blob += c.read_text(errors='ignore').lower()
        except: pass

    orphans = []
    for a in assets:
        name = a.name.lower()
        if name not in big_code_blob and a.stem.lower() not in big_code_blob:
            orphans.append(a.relative_to(project_path))

    # 2. DEEP FORENSICS & VFS AUDIT
    all_leaks = []
    all_missing = []
    
    # 2a. P3D Scans
    for a in assets:
        if a.suffix.lower() == ".p3d":
            forensic_results, textures = audit_p3d(a, project_path)
            for r in forensic_results: print(r)
            
            leaks, missing = validate_vfs_links(textures, project_path)
            all_leaks.extend([(a.name, l) for l in leaks])
            all_missing.extend([(a.name, m) for m in missing])

    # 2b. Source Code Scans (Check for leaks in config/scripts)
    path_regex = re.compile(r'[\'"]\\?([a-zA-Z0-9_][a-zA-Z0-9_\\.-]+\.(paa|rvmat|p3d))[\'"]', re.IGNORECASE)
    for c in code_files:
        try:
            content = c.read_text(errors='ignore')
            matches = path_regex.findall(content)
            # Findall returns tuples if there are multiple groups, we want group 1 (the path)
            code_paths = [m[0] for m in matches]
            
            leaks, missing = validate_vfs_links(code_paths, project_path)
            all_leaks.extend([(c.name, l) for l in leaks])
            all_missing.extend([(c.name, m) for m in missing])
        except: pass

    # 3. SUMMARY REPORT
    print("\n[Summary Report]")
    
    if orphans:
        print(f"  ❌ {len(orphans)} Orphaned Assets (Unused in code)")
    else:
        print("  ✅ Reference Integrity: PASS")

    if all_leaks:
        # Deduplicate leaks
        unique_leaks = sorted(list(set(all_leaks)))
        print(f"  ⚠️  {len(unique_leaks)} External Leaks Detected (Non-unit paths):")
        for p3d, leak in unique_leaks[:10]:
            print(f"     - {p3d} -> {leak}")
    else:
        print("  ✅ External Leakage: NONE")

    if all_missing:
        unique_missing = sorted(list(set(all_missing)))
        print(f"  ❌ {len(unique_missing)} Missing Internal Links (Dead VFS paths):")
        for p3d, miss in unique_missing[:10]:
            print(f"     - {p3d} -> {miss}")
    else:
        print("  ✅ VFS Link Integrity: PASS")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: audit_assets.py <project_path>")
        sys.exit(1)
    audit_project_assets(sys.argv[1])
