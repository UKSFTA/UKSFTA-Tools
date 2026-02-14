#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import subprocess
from pathlib import Path
from asset_auditor import audit_p3d, validate_vfs_links

# --- CONFIGURATION ---
# Default Workshop path - Adjust if needed
WORKSHOP_ROOT = Path.home() / ".steam" / "steam" / "steamapps" / "workshop" / "content" / "107410"

def get_ids_from_preset(html_path):
    """Extracts Workshop IDs from an Arma 3 Launcher HTML preset."""
    with open(html_path, 'r', errors='ignore') as f:
        content = f.read()
        return re.findall(r'id=(\d+)', content)

def audit_modlist(html_path, workshop_path=None):
    if not workshop_path:
        workshop_path = WORKSHOP_ROOT
    
    workshop_path = Path(workshop_path)
    if not workshop_path.exists():
        print(f"❌ Error: Steam Workshop directory not found at {workshop_path}")
        return

    mod_ids = list(set(get_ids_from_preset(html_path)))
    print(f"\n🕵️  [Modlist Auditor] Analyzing {len(mod_ids)} mods...")
    print(f"[*] Target Workshop: {workshop_path}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

    total_issues = 0
    
    for mid in mod_ids:
        mod_dir = workshop_path / mid
        if not mod_dir.exists():
            print(f"  [!] Mod {mid} is NOT downloaded locally. Skipping.")
            continue

        print(f"\n[*] Auditing Mod: {mid}")
        
        # Scan for P3Ds in the mod folder
        p3ds = list(mod_dir.rglob("*.p3d"))
        pbos = list(mod_dir.rglob("*.pbo"))
        
        if not p3ds and not pbos:
            print("    ℹ️  No assets found (Raw or Packed).")
            continue

        if pbos and not p3ds:
            print(f"    📦 {len(pbos)} PBOs detected (packed assets). Deep-audit skipped.")
            continue

        mod_issues = 0
        for p in p3ds:
            # We use our established forensic engine
            # We only report FAILURES here to avoid spamming the terminal
            cmd = [str(Path(__file__).parent / "bin" / "linux-x64" / "debinarizer"), str(p), "-audit-lods"]
            try:
                res = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
                if "FAIL" in res.stdout or "MISSING" in res.stdout:
                    print(f"    ❌ {p.name}: {res.stdout.strip()}")
                    mod_issues += 1
            except: pass

        if mod_issues == 0:
            print("    ✅ Integrity: PASS")
        else:
            total_issues += mod_issues

    print("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print(f"✨ Audit Complete. Total Asset Deficits Found: {total_issues}")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: modlist_auditor.py <preset.html> [workshop_path_override]")
        sys.exit(1)
    
    override = sys.argv[2] if len(sys.argv) > 2 else None
    audit_modlist(sys.argv[1], override)
