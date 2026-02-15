#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import re
import sys
from pathlib import Path

# --- CONFIGURATION ---
# Flexible regex to find class definitions
CLASS_PATTERN = re.compile(r'class\s+([a-zA-Z0-9_]+)\s*:\s*([a-zA-Z0-9_]+)', re.IGNORECASE)

def generate_ace_config(config_path):
    path = Path(config_path)
    if not path.exists():
        print(f"❌ File not found: {config_path}")
        return

    header = "
🎒 [ACE Arsenal Helper] Processing: " + path.name
    separator = " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print(header); print(separator)

    try:
        content = path.read_text(errors='ignore')
    except Exception as e:
        print(f"❌ Error reading file: {e}"); return

    matches = CLASS_PATTERN.findall(content)
    hierarchy = {} 
    
    for child, parent in matches:
        if parent not in hierarchy: hierarchy[parent] = []
        hierarchy[parent].append(child)

    IGNORE_BASES = {"Vest_Camo_Base", "H_HelmetB", "B_Soldier_F", "Uniform_Base", "ItemCore", "VestItem"}
    ace_groups = []
    
    for parent, children in hierarchy.items():
        if parent in IGNORE_BASES: continue
        if len(children) > 1:
            ace_groups.append({"name": "Group_" + parent, "base": parent, "variants": children})

    if not ace_groups:
        print("  ℹ️  No variant groups detected."); return

    output = ["
class ACE_Arsenal_Config {"]
    for g in ace_groups:
        output.append("    class " + g['name'] + " {")
        output.append('        base = "' + g["base"] + '";')
        variants_str = ", ".join(['"' + v + '"' for v in g["variants"]])
        output.append("        variants[] = {" + variants_str + "};")
        output.append("    };")
    output.append("};
")

    print("
".join(output))
    print(separator)
    print(f"✨ ACE Extended Arsenal config generated for {len(ace_groups)} groups.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: ace_arsenal_helper.py <config.cpp>"); sys.exit(1)
    generate_ace_config(sys.argv[1])
