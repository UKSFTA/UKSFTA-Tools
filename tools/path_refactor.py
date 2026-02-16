#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
from pathlib import Path

def refactor_paths(project_path, old_tag, new_prefix):
    r"""
    Deep byte-level refactor of paths in binary and text files.
    Accounts for various case variations and ensures VFS standard compliance.
    """
    project_path = Path(project_path)
    print(f"\n[*] Global Deep Refactor: {project_path.name}")
    print(f"[*] Targeting: {old_tag} -> {new_prefix}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

    # Target all relevant file types, especially P3D and RVMAT
    exts = {".cpp", ".hpp", ".sqf", ".rvmat", ".ext", ".cfg", ".p3d", ".wrp"}
    count = 0
    
    # Generate common variations of the search tag to handle inconsistent casing in old files
    variations = [
        old_tag,
        old_tag.lower(),
        old_tag.upper(),
        old_tag.capitalize()
    ]
    # Deduplicate variations
    search_bytes = [v.encode('utf-8') for v in sorted(list(set(variations)))]
    replace_byte = new_prefix.encode('utf-8')

    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            path = Path(root) / f
            if path.suffix.lower() in exts:
                try:
                    with open(path, 'rb') as f_in:
                        content = f_in.read()
                    
                    original_content = content
                    for s_byte in search_bytes:
                        if s_byte in content:
                            content = content.replace(s_byte, replace_byte)
                    
                    if content != original_content:
                        with open(path, 'wb') as f_out:
                            f_out.write(content)
                        print(f"  ✅ Refactored: {path.relative_to(project_path)}")
                        count += 1
                except Exception as e:
                    print(f"  ❌ Error processing {f}: {e}")

    print(f"\n✨ Deep Refactor complete. Updated {count} files.")

if __name__ == "__main__":
    if len(sys.argv) < 4:
        print("Usage: path_refactor.py <project_path> <old_string> <new_string>")
        sys.exit(1)
    
    refactor_paths(sys.argv[1], sys.argv[2], sys.argv[3])
