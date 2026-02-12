#!/usr/bin/env python3
import os
import sys
import re
import xml.etree.ElementTree as ET
from pathlib import Path

# UKSFTA Localization Vacuum
# Purges unused keys from stringtable.xml based on code usage.

def clean_project_strings(project_path):
    root = Path(project_path)
    print(f"🧹 Vacuuming Strings for: {root.name}")
    
    st_path = root / "addons" / "main" / "stringtable.xml"
    if not st_path.exists():
        print("  i No stringtable.xml found in addons/main/")
        return

    # 1. Collect all used keys from code (.sqf, .cpp, .hpp)
    used_keys = set()
    code_exts = {".sqf", ".cpp", ".hpp"}
    
    for f in root.rglob("*"):
        if f.suffix.lower() in code_exts and ".git" not in str(f) and ".hemttout" not in str(f):
            try:
                content = f.read_text(errors='ignore')
                # Find STR_ prefix keys (standard Arma convention)
                matches = re.findall(r'STR_[A-Za-z0-9_]+', content)
                for m in matches: used_keys.add(m)
            except: pass

    # 2. Parse XML and find orphaned keys
    try:
        tree = ET.parse(st_path)
        xml_root = tree.getroot()
        
        to_remove = []
        key_count = 0
        removed_count = 0

        # Arma stringtables use Key tags
        for package in xml_root.findall('.//Package'):
            for container in package.findall('.//Container'):
                for key in container.findall('.//Key'):
                    key_count += 1
                    key_id = key.get('ID')
                    if key_id not in used_keys:
                        container.remove(key)
                        removed_count += 1
        
        if removed_count > 0:
            tree.write(st_path, encoding='utf-8', xml_declaration=True)
            print(f"  ✅ Removed {removed_count} orphaned keys (Total keys: {key_count - removed_count})")
        else:
            print(f"  ✅ All {key_count} keys are in active use.")
            
    except Exception as e:
        print(f"  ❌ Error processing XML: {e}")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: string_cleaner.py <project_path>")
        sys.exit(1)
    clean_project_strings(sys.argv[1])
