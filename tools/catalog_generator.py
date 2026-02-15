#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import re
import json
import sys
from pathlib import Path
from datetime import datetime

# --- CONFIGURATION ---
CLASS_PATTERN = re.compile(r'class\s+([a-zA-Z0-9_]+)\s*:\s*([a-zA-Z0-9_]+)', re.IGNORECASE)
DISPLAY_NAME_PATTERN = re.compile(r'displayName\s*=\s*"([^"]+)"', re.IGNORECASE)
SCOPE_PATTERN = re.compile(r'scope\s*=\s*(\d)', re.IGNORECASE)
AUTHOR_PATTERN = re.compile(r'author\s*=\s*"([^"]+)"', re.IGNORECASE)

def get_projects():
    parent_dir = Path(__file__).parent.parent.parent.resolve()
    return sorted([d for d in parent_dir.iterdir() if d.is_dir() and d.name.startswith("UKSFTA-")])

def parse_config(config_path):
    try: content = config_path.read_text(errors='ignore')
    except: return []
    assets = []; blocks = re.split(r'class\s+', content)
    for block in blocks[1:]:
        full_block = "class " + block
        header_match = CLASS_PATTERN.search(full_block)
        if not header_match: continue
        classname = header_match.group(1); parent = header_match.group(2)
        scope_match = SCOPE_PATTERN.search(full_block)
        scope = int(scope_match.group(1)) if scope_match else 0
        if scope != 2: continue
        display_name_match = DISPLAY_NAME_PATTERN.search(full_block)
        display_name = display_name_match.group(1) if display_name_match else classname
        author_match = AUTHOR_PATTERN.search(full_block)
        author = author_match.group(1) if author_match else "UKSFTA"
        assets.append({"className": classname, "displayName": display_name, "parent": parent, "author": author, "project": config_path.parent.parent.parent.name})
    return assets

def generate_catalog():
    projects = get_projects(); all_assets = []
    header = "\n🛡️  [Virtual Armory] Indexing Unit Assets..."
    separator = " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print(header); print(separator)
    for p in projects:
        configs = list(p.rglob("config.cpp"))
        for cfg in configs:
            if ".hemttout" in str(cfg): continue
            found = parse_config(cfg)
            if found:
                print("  ✅ " + p.name + ": Found " + str(len(found)) + " assets.")
                all_assets.extend(found)
    if not all_assets: print("  ❌ No assets found in workspace."); return
    output_json = Path(__file__).parent.parent / "unit_assets.json"
    with open(output_json, 'w') as f: json.dump(all_assets, f, indent=2)
    output_md = Path(__file__).parent.parent / "ASSET_CATALOG.md"
    now = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    md_lines = ["# 🛡️ UKSFTA Virtual Armory", "\n*Generated on: " + now + "*", "\n| Display Name | Class Name | Author | Project |", "| :--- | :--- | :--- | :--- |"]
    for a in sorted(all_assets, key=lambda x: (x['project'], x['displayName'])):
        md_lines.append("| " + a['displayName'] + " | `" + a['className'] + "` | " + a['author'] + " | " + a['project'] + " |")
    output_md.write_text("\n".join(md_lines))
    print(separator)
    print("✨ Catalog Complete: " + str(len(all_assets)) + " assets indexed.")

if __name__ == "__main__": generate_catalog()
