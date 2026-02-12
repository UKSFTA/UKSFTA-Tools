#!/usr/bin/env python3
import os
import sys
from pathlib import Path

# UKSFTA Code Standard Enforcer
# Normalizes indentation (4 spaces), strips trailing whitespace, ensures EOF newline.

def fix_file(file_path):
    try:
        with open(file_path, 'r', encoding='utf-8', errors='ignore') as f:
            lines = f.readlines()
        
        fixed_lines = []
        for line in lines:
            # 1. Convert Tabs to 4 Spaces
            line = line.replace('	', '    ')
            # 2. Strip Trailing Whitespace
            line = line.rstrip()
            fixed_lines.append(line)
        
        # 3. Ensure EOF Newline
        content = "
".join(fixed_lines).strip() + "
"
        
        with open(file_path, 'w', encoding='utf-8') as f:
            f.write(content)
        return True
    except:
        return False

def main(target_dir):
    root = Path(target_dir)
    print(f"🧹 Fixing syntax in: {root.name}")
    
    exts = {".cpp", ".hpp", ".sqf", ".rhai", ".toml", ".txt"}
    count = 0
    for file in root.rglob("*"):
        if file.suffix.lower() in exts and ".hemttout" not in str(file) and ".git" not in str(file):
            if fix_file(file):
                count += 1
    
    print(f"  ✅ Standardized {count} files.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: syntax_fixer.py <target_directory>")
        sys.exit(1)
    main(sys.argv[1])
