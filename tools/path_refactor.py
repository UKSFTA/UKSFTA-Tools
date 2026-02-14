#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
from pathlib import Path

# --- CONFIGURATION ---
UNIT_PREFIX = r"z\uksfta\addons"

def get_project_vfs_path(project_path):
    r"""Derives the VFS path for a project, e.g. z\uksfta\addons\maps"""
    name = Path(project_path).name.lower()
    if name.startswith("uksfta-"):
        name = name.replace("uksfta-", "")
    return UNIT_PREFIX + "\\" + name

def refactor_paths(project_path, old_tag, new_prefix=None):
    r"""
    Scans code files and replaces legacy paths.
    Uses literal replacement to avoid Unicode escape issues with Arma paths like \u...
    """
    project_path = Path(project_path)
    if not new_prefix:
        new_prefix = get_project_vfs_path(project_path)

    print(f"\n[*] Global Refactor: {project_path.name}")
    print(f"[*] Targeting: \\{old_tag}\\ -> \\{new_prefix}\\")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

    exts = {".cpp", ".hpp", ".sqf", ".rvmat", ".ext", ".cfg"}
    count = 0
    
    # Normalize tags for search
    # We look for \old_tag\ or old_tag\
    search_a = "\\" + old_tag.strip("\\") + "\\"
    search_b = old_tag.strip("\\") + "\\"
    
    replace_a = "\\" + new_prefix.strip("\\") + "\\"
    replace_b = new_prefix.strip("\\") + "\\"

    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            path = Path(root) / f
            if path.suffix.lower() in exts:
                try:
                    # Read as binary to completely bypass Python string escape processing
                    with open(path, 'rb') as f_in:
                        content = f_in.read()
                    
                    changed = False
                    # Perform literal byte replacement
                    if search_a.encode('utf-8') in content:
                        content = content.replace(search_a.encode('utf-8'), replace_a.encode('utf-8'))
                        changed = True
                    if search_b.encode('utf-8') in content:
                        content = content.replace(search_b.encode('utf-8'), replace_b.encode('utf-8'))
                        changed = True
                        
                    if changed:
                        with open(path, 'wb') as f_out:
                            f_out.write(content)
                        print(f"  ✅ Refactored: {path.relative_to(project_path)}")
                        count += 1
                except Exception as e:
                    print(f"  ❌ Error processing {f}: {e}")

    print(f"\n✨ Refactor complete. Updated {count} files.")

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: path_refactor.py <project_path> <old_tag_to_replace> [new_prefix_override]")
        sys.exit(1)
    
    override = sys.argv[3] if len(sys.argv) > 3 else None
    refactor_paths(sys.argv[1], sys.argv[2], override)
