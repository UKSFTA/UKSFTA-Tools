#!/usr/bin/env python3
import os
import sys
import argparse
from pathlib import Path

# UKSFTA Code Standard Enforcer
# Normalizes indentation (4 spaces), strips trailing whitespace, ensures EOF newline.

def fix_file(file_path, dry_run=False):
    try:
        with open(file_path, 'r', encoding='utf-8', errors='ignore') as f:
            content = f.read()
        
        # 1. Convert Tabs to 4 Spaces
        # 2. Strip Trailing Whitespace
        lines = [line.replace('\t', '    ').rstrip() for line in content.splitlines()]
        
        # 3. Ensure EOF Newline
        fixed_content = "\n".join(lines).strip() + "\n"
        
        if content == fixed_content:
            return False, False # No changes needed

        if not dry_run:
            with open(file_path, 'w', encoding='utf-8') as f:
                f.write(fixed_content)
        return True, True
    except:
        return False, False

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Syntax Fixer")
    parser.add_argument("target", help="Target directory")
    parser.add_argument("--dry-run", action="store_true", help="Preview changes without writing")
    args = parser.parse_args()

    root = Path(args.target)
    mode_str = "[DRY-RUN] " if args.dry_run else ""
    print(f"🧹 {mode_str}Standardizing syntax in: {root.name}")
    
    exts = {".cpp", ".hpp", ".sqf", ".rhai", ".toml", ".txt", ".ext"}
    count = 0
    for file in root.rglob("*"):
        if file.suffix.lower() in exts and ".hemttout" not in str(file) and ".git" not in str(file):
            changed, success = fix_file(file, args.dry_run)
            if changed:
                if args.dry_run: print(f"  [DRY-RUN] Would fix: {file.relative_to(root)}")
                count += 1
    
    msg = "Would standardize" if args.dry_run else "Standardized"
    print(f"  ✅ {msg} {count} files.")

if __name__ == "__main__":
    main()
