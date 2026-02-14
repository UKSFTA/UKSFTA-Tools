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

def parse_p3d_info(p3d_path):
    """Parses structural metadata from the forensic binary."""
    if not DEBINARIZER.exists():
        return None

    cmd = [str(DEBINARIZER), str(p3d_path), "-info"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
        output = res.stdout
        
        data = {
            "mass": 0.0,
            "lods": {}, # Resolution -> Vertex Count
            "textures": set(),
            "selections": set(),
            "proxies": set()
        }

        # Parse Mass
        mass_m = re.search(r"Mass:\s+([\d\.]+)", output)
        if mass_m: data["mass"] = float(mass_m.group(1))

        # Parse LODs
        # Format: "    - 1.0: 1234 pts, 2 textures"
        lod_matches = re.finditer(r"-\s+([\d\.E\+\-]+):\s+(\d+)\s+pts", output)
        for m in lod_matches:
            data["lods"][m.group(1)] = int(m.group(2))

        # Parse Sections
        def extract_section(section_name, blob):
            # Find the section header and grab everything until the next section header or end of string
            pattern = re.escape(f"[{section_name}]") + r"(.*?)(?=\n\s*\[|$)"
            m = re.search(pattern, blob, re.DOTALL)
            if m:
                lines = m.group(1).strip().splitlines()
                return {l.strip()[1:].strip() for l in lines if l.strip().startswith("-")}
            return set()

        data["textures"] = extract_section("VFS Links", output)
        data["selections"] = extract_section("Named Selections", output)
        data["proxies"] = extract_section("Proxies", output)

        return data
    except:
        return None

def compare_assets(path_a, path_b):
    info_a = parse_p3d_info(path_a)
    info_b = parse_p3d_info(path_b)

    if not info_a or not info_b:
        print("❌ Error: Failed to parse one or both assets.")
        return

    print(f"\n🔍 [Asset Comparison] {os.path.basename(path_a)} vs {os.path.basename(path_b)}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

    # 1. Mass
    diff_mass = info_b["mass"] - info_a["mass"]
    status = "CHANGED" if abs(diff_mass) > 0.01 else "IDENTICAL"
    print(f"  Mass: {info_a['mass']:.2f} -> {info_b['mass']:.2f} ({status})")

    # 2. LODs
    all_res_keys = set(info_a["lods"].keys()) | set(info_b["lods"].keys())
    
    def res_to_float(r):
        try: return float(r)
        except: return 0.0

    sorted_res = sorted(list(all_res_keys), key=res_to_float)
    
    print("\n  [LOD Integrity]")
    for res in sorted_res:
        v_a = info_a["lods"].get(res, 0)
        v_b = info_b["lods"].get(res, 0)
        marker = " "
        if v_a == 0: marker = "➕"
        elif v_b == 0: marker = "➖"
        elif v_a != v_b: marker = "⚡"
        
        print(f"    {marker} LOD {res:<10} | Vertices: {v_a:>6} -> {v_b:<6}")

    # 3. Structural Comparison
    def print_structural_diff(label, set_a, set_b):
        added = set_b - set_a
        removed = set_a - set_b
        print(f"\n  [{label}]")
        if not added and not removed:
            print("    ✅ Identical")
            return
        
        for r in sorted(removed): print(f"    ➖ {r}")
        for a in sorted(added):   print(f"    ➕ {a}")

    print_structural_diff("VFS Links", info_a["textures"], info_b["textures"])
    print_structural_diff("Named Selections", info_a["selections"], info_b["selections"])
    print_structural_diff("Proxies", info_a["proxies"], info_b["proxies"])
    print("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: model_diff.py <file_a.p3d> <file_b.p3d>")
        sys.exit(1)
    compare_assets(sys.argv[1], sys.argv[2])
