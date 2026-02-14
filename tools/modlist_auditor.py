#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import subprocess
import shutil
import tempfile
from pathlib import Path

# --- CONFIGURATION ---
TOOLS_ROOT = Path(__file__).parent.parent
DEBINARIZER = TOOLS_ROOT / "bin" / "linux-x64" / "debinarizer"
if sys.platform == "win32":
    DEBINARIZER = TOOLS_ROOT / "bin" / "win-x64" / "debinarizer.exe"

# Default Workshop path
WORKSHOP_ROOT = Path("/ext/SteamLibrary/steamapps/workshop/content/107410")

def get_ids_from_preset(html_path):
    """Extracts Workshop IDs from an Arma 3 Launcher HTML preset."""
    with open(html_path, 'r', errors='ignore') as f:
        content = f.read()
        return re.findall(r'id=(\d+)', content)

def audit_p3d_file(p3d_path):
    """Runs the forensic audit on a single P3D file."""
    if not DEBINARIZER.exists():
        return False, "Forensic binary missing"
    
    cmd = [str(DEBINARIZER), str(p3d_path), "-audit-lods"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
        if "FAIL" in res.stdout or "MISSING" in res.stdout:
            return True, res.stdout.strip()
    except: pass
    return False, ""

def audit_modlist(html_path, workshop_path=None, deep_scan=False):
    if not workshop_path:
        workshop_path = WORKSHOP_ROOT
    
    workshop_path = Path(workshop_path)
    if not workshop_path.exists():
        print(f"❌ Error: Steam Workshop directory not found at {workshop_path}")
        return

    mod_ids = list(set(get_ids_from_preset(html_path)))
    print(f"\n🕵️  [Modlist Auditor] Analyzing {len(mod_ids)} mods...")
    if deep_scan: print("🔍 DEEP SCAN ENABLED: Unpacking PBOs for forensic inspection.")
    print(f"[*] Target Workshop: {workshop_path}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

    total_issues = 0
    
    for mid in mod_ids:
        mod_dir = workshop_path / mid
        if not mod_dir.exists():
            print(f"  [!] Mod {mid} is NOT downloaded locally. Skipping.")
            continue

        print(f"\n[*] Auditing Mod: {mid}")
        
        # 1. Audit loose P3Ds
        p3ds = list(mod_dir.rglob("*.p3d"))
        mod_issues = 0
        
        for p in p3ds:
            has_issue, detail = audit_p3d_file(p)
            if has_issue:
                print(f"    ❌ {p.name}: {detail}")
                mod_issues += 1

        # 2. Audit Packed PBOs (if deep scan enabled)
        if deep_scan:
            pbos = list(mod_dir.rglob("*.pbo"))
            if pbos:
                print(f"    📦 Cracking {len(pbos)} PBOs...")
                with tempfile.TemporaryDirectory(prefix="uksfta_audit_") as tmpdir:
                    for pbo in pbos:
                        try:
                            # Use correct hemtt command to unpack
                            # hemtt utils pbo unpack <PBO> [OUTPUT]
                            subprocess.run(["hemtt", "utils", "pbo", "unpack", str(pbo), tmpdir], 
                                         capture_output=True, check=True)
                            
                            # Audit extracted P3Ds
                            extracted_p3ds = list(Path(tmpdir).rglob("*.p3d"))
                            for ep in extracted_p3ds:
                                has_issue, detail = audit_p3d_file(ep)
                                if has_issue:
                                    # Format output to show internal path if possible
                                    rel_ep = ep.relative_to(tmpdir)
                                    print(f"    ❌ {pbo.name} > {rel_ep}: {detail}")
                                    mod_issues += 1
                            
                            # Clear tmpdir for next PBO to save space
                            for item in os.listdir(tmpdir):
                                item_path = os.path.join(tmpdir, item)
                                if os.path.isdir(item_path): shutil.rmtree(item_path)
                                else: os.remove(item_path)
                                
                        except Exception as e:
                            print(f"    ⚠️  Failed to extract {pbo.name}: {e}")

        if mod_issues == 0:
            print("    ✅ Integrity: PASS")
        else:
            total_issues += mod_issues

    print("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print(f"✨ Audit Complete. Total Asset Deficits Found: {total_issues}")

if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description="UKSFTA Modlist Auditor")
    parser.add_argument("file", help="Launcher preset HTML")
    parser.add_argument("--workshop", help="Steam Workshop path override")
    parser.add_argument("--deep", action="store_true", help="Extract PBOs for forensic audit")
    
    args = parser.parse_args()
    audit_modlist(args.file, args.workshop, args.deep)
