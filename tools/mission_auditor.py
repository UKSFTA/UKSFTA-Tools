#!/usr/bin/env python3
import os
import sys
import re
import subprocess
import shutil
from pathlib import Path

def get_mission_addons(pbo_path, temp_dir):
    """Extract and parse mission.sqm for required addons."""
    sqm_path = Path(temp_dir) / "mission.sqm"
    
    # 1. Extract mission.sqm from PBO
    # -P extracts a specific file
    try:
        subprocess.run(["extractpbo", "-P", pbo_path, "mission.sqm", str(temp_dir)], check=True, stdout=subprocess.DEVNULL)
    except:
        return None

    if not sqm_path.exists():
        return None

    # 2. Derap if binary
    try:
        subprocess.run(["derap", str(sqm_path)], check=True, stdout=subprocess.DEVNULL)
    except:
        pass # Might already be text

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
    
    temp_dir = Path("/tmp/uksfta_audit")
    if temp_dir.exists(): shutil.rmtree(temp_dir)
    temp_dir.mkdir(parents=True)

    required = get_mission_addons(pbo_path, temp_dir)
    
    if required is None:
        print("  ❌ Error: Could not extract or parse mission.sqm from PBO.")
        return

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

    # Return results for the Rich Table in workspace_manager
    return {
        "required": required,
        "local": resolved_local,
        "external": resolved_external,
        "missing": missing
    }

if __name__ == "__main__":
    # Test stub
    pass
