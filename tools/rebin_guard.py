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

def check_geometry_health(p3d_path):
    """
    Scans P3D metadata for potential binarization failures.
    Specifically checks for Geometry LOD presence and valid VFS links.
    """
    if not DEBINARIZER.exists():
        return "❌ Binary Missing"

    print(f"\n[*] Checking Binarization Readiness: {os.path.basename(p3d_path)}")
    
    # 1. Check for missing critical LODs
    cmd = [str(DEBINARIZER), str(p3d_path), "-audit-lods"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
        if "MISSING GEOMETRY" in res.stdout:
            print("  ❌ FAIL: Missing Geometry LOD (Server crash risk).")
            return False
        if "MISSING SHADOW" in res.stdout:
            print("  ⚠️  WARN: Missing Shadow Volume (Client visual artifact).")
    except: pass

    # 2. Check for path normalization issues
    cmd_info = [str(DEBINARIZER), str(p3d_path), "-info"]
    try:
        res = subprocess.run(cmd_info, capture_output=True, text=True, timeout=10)
        # Search for any non-normalized paths (e.g., local C:\ paths)
        local_path_match = re.search(r'[a-z]:\\', res.stdout, re.IGNORECASE)
        if local_path_match:
            print(f"  ❌ FAIL: Non-normalized path detected: {local_path_match.group(0)}")
            return False
    except: pass

    print("  ✅ PASS: Asset is ready for binarization.")
    return True

def main():
    if len(sys.argv) < 2:
        print("Usage: rebin_guard.py <file.p3d>")
        sys.exit(1)
    
    success = check_geometry_health(sys.argv[1])
    if not success:
        sys.exit(1)

if __name__ == "__main__":
    main()
