#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
from pathlib import Path

# --- CONFIGURATION ---
UNIT_PREFIX = r"z\uksfta\addons"

def sanitize_content(content):
    r"""
    Finds recursive VFS patterns and collapses them.
    Example: z\z\z\uksfta\addons\mods\addons\mods\addons\main -> z\uksfta\addons\mods\main
    """
    # 1. Collapse multiple z\
    # Note: Using raw strings for regex to handle backslashes safely
    content = re.sub(r'(\\?z\\)+', r'z\\', content, flags=re.IGNORECASE)
    
    # 2. Fix redundant project/addons nesting
    # Strategy: replace 'addons\PROJECT\addons' with 'addons'
    # We loop to catch multiple levels of recursion
    for _ in range(4):
        # This matches: addons\word\addons and replaces with addons
        content = re.sub(r'addons\\[a-zA-Z0-9_]+\\addons', r'addons', content, flags=re.IGNORECASE)
    
    # 3. Final cleanup of any double backslashes
    content = content.replace('\\\\', '\\')
    
    return content

def sanitize_project(project_path):
    print(f"\n✨ [VFS Sanitizer] Repairing: {Path(project_path).name}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    
    exts = {".cpp", ".hpp", ".sqf", ".rvmat", ".ext", ".cfg"}
    count = 0
    
    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            path = Path(root) / f
            if path.suffix.lower() in exts:
                try:
                    with open(path, 'rb') as f_in:
                        data = f_in.read()
                    
                    try:
                        content = data.decode('utf-8')
                    except:
                        content = data.decode('latin-1')
                        
                    new_content = sanitize_content(content)
                    
                    if new_content != content:
                        with open(path, 'w', encoding='utf-8') as f_out:
                            f_out.write(new_content)
                        print(f"  ✅ Repaired: {path.relative_to(project_path)}")
                        count += 1
                except Exception as e:
                    print(f"  ❌ Error processing {f}: {e}")

    print(f"\n✨ Sanitization complete. Fixed {count} files.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: vfs_sanitizer.py <project_path>")
        sys.exit(1)
    sanitize_project(sys.argv[1])
