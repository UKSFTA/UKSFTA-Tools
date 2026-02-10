#!/usr/bin/env python3
import os
import sys
import re
import subprocess
import shutil
from pathlib import Path

def get_mission_addons(pbo_path, temp_dir):
    """Extract and parse mission.sqm for required addons."""
    pbo_path = os.path.abspath(pbo_path)
    temp_dir = os.path.abspath(temp_dir)
    
    # Clean temp dir
    if os.path.exists(temp_dir):
        shutil.rmtree(temp_dir)
    os.makedirs(temp_dir)

    # 1. Extract mission.sqm using Mikero's -F flag
    # Note: ExtractPBO on Linux/Wine often expects absolute paths
    try:
        # We try to extract mission.sqm specifically
        subprocess.run(["extractpbo", "-P", f"-F=mission.sqm", pbo_path, temp_dir], check=True, stdout=subprocess.DEVNULL)
    except subprocess.CalledProcessError:
        # Try case-sensitive alternative just in case
        try:
            subprocess.run(["extractpbo", "-P", f"-F=Mission.sqm", pbo_path, temp_dir], check=True, stdout=subprocess.DEVNULL)
        except:
            return None

    # Find the extracted file (might be nested)
    sqm_path = None
    for root, _, files in os.walk(temp_dir):
        for f in files:
            if f.lower() == "mission.sqm":
                sqm_path = Path(root) / f
                break
    
    if not sqm_path or not sqm_path.exists():
        return None

    # 2. Derap if binary
    try:
        subprocess.run(["derap", str(sqm_path)], check=True, stdout=subprocess.DEVNULL)
    except:
        pass 

    # 3. Parse Addons
    content = sqm_path.read_text(errors='ignore')
    addons = set()
    
    # Match addons[]={...}; and addonsAuto[]={...};
    matches = re.finditer(r'addons(?:Auto)?\[\]\s*=\s*\{([^}]*)\}', content, re.MULTILINE | re.DOTALL)
    for m in matches:
        items = [i.strip().replace('"', '').replace("'", "") for i in m.group(1).split(',')]
        for i in items:
            if i: addons.add(i)
            
    return sorted(list(addons))

def audit_mission(pbo_path, local_patches):
    print(f"🔮 Auditing Mission: {os.path.basename(pbo_path)}")
    
    temp_dir = "/tmp/uksfta_audit"
    required = get_mission_addons(pbo_path, temp_dir)
    
    if required is None:
        print("  ❌ Error: Could not extract or parse mission.sqm. Verify extractpbo is installed and path is valid.")
        return None

    known_externals = ["A3_", "cba_", "ace_", "task_force_radio", "acre_", "rhsusf_", "rhs_", "cup_", "uk3cb_"]
    
    missing = []
    resolved_local = []
    resolved_external = []

    for req in required:
        if req in local_patches:
            resolved_local.append(req)
        elif any(req.lower().startswith(ext.lower()) for ext in known_externals):
            resolved_external.append(req)
        else:
            missing.append(req)

    return {
        "required": required,
        "local": resolved_local,
        "external": resolved_external,
        "missing": missing
    }

if __name__ == "__main__":
    pass
