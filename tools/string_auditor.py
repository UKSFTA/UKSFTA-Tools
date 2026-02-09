#!/usr/bin/env python3
import os
import sys
import re
from pathlib import Path
import xml.etree.ElementTree as ET

def audit_strings(project_path):
    print(f"🌍 Auditing Localization for: {os.path.basename(project_path)}")
    
    st_path = Path(project_path) / "addons" / "main" / "stringtable.xml"
    if not st_path.exists():
        # Try finding ANY stringtable in addons
        st_paths = list(Path(project_path).glob("addons/*/stringtable.xml"))
        if not st_paths:
            print("  No stringtable.xml found.")
            return
        st_path = st_paths[0]

    # 1. Extract keys from XML
    xml_keys = set()
    try:
        tree = ET.parse(st_path)
        for key in tree.iter('Key'):
            xml_keys.add(key.get('ID').lower())
    except Exception as e:
        print(f"  ❌ Error parsing XML: {e}")
        return

    # 2. Extract keys from Code
    code_keys = set()
    code_exts = {".cpp", ".hpp", ".sqf"}
    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            if Path(f).suffix.lower() in code_exts:
                content = (Path(root) / f).read_text(errors='ignore')
                # Match STR_UKSTFA_Something
                matches = re.findall(r'STR_[a-zA-Z0-9_]+', content)
                for m in matches:
                    code_keys.add(m.lower())

    # 3. Compare
    missing_in_xml = code_keys - xml_keys
    unused_in_xml = xml_keys - code_keys

    if missing_in_xml:
        print(f"  ❌ {len(missing_in_xml)} keys used in code but MISSING in stringtable:")
        for k in sorted(missing_in_xml):
            print(f"     - {k.upper()}")
    
    if unused_in_xml:
        print(f"  ⚠️ {len(unused_in_xml)} keys exist in stringtable but are UNUSED in code:")
        for k in sorted(unused_in_xml):
            print(f"     - {k.upper()}")

    if not missing_in_xml and not unused_in_xml:
        print("  ✅ Localization is 100% synchronized.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: string_auditor.py <project_path>")
        sys.exit(1)
    audit_strings(sys.argv[1])
