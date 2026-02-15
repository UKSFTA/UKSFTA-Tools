#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import argparse
import fnmatch
from pathlib import Path

def validate_sqf(filepath):
    """Simple syntax check for common SQF errors."""
    bad_count = 0
    try:
        with open(filepath, 'r', encoding='utf-8', errors='ignore') as f:
            content = f.read()
            # Basic validation: check for balanced brackets
            for char_open, char_close in [('(', ')'), ('[', ']'), ('{', '}')]:
                if content.count(char_open) != content.count(char_close):
                    print(f"ERROR: Unbalanced {char_open}{char_close} in {filepath}")
                    bad_count += 1
    except Exception as e:
        print(f"ERROR: Failed to read {filepath}: {e}")
        bad_count += 1
    return bad_count

def main():
    print("Validating SQF")
    parser = argparse.ArgumentParser()
    parser.add_argument('-m','--module', help='only search specified module addon folder', required=False, default="")
    parser.add_argument('path', nargs='?', default=".", help="Project path to scan")
    args = parser.parse_args()

    bad_count = 0
    sqf_files = []
    
    scan_root = Path(args.path)
    addons_dir = scan_root / "addons"
    target_dir = addons_dir if addons_dir.exists() else scan_root
    
    if args.module:
        target_dir = target_dir / args.module

    if not target_dir.exists():
        print(f"  [!] Target directory not found: {target_dir}")
        return 0

    for root, _, filenames in os.walk(target_dir):
        if ".hemttout" in root or ".uksf_tools" in root: continue
        for filename in filenames:
            if filename.lower().endswith('.sqf'):
                sqf_files.append(os.path.join(root, filename))

    for f in sqf_files:
        bad_count += validate_sqf(f)

    print(f"------\nChecked {len(sqf_files)} files\nErrors detected: {bad_count}")
    return bad_count

if __name__ == "__main__":
    sys.exit(main())
