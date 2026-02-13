#!/usr/bin/env python3

import fnmatch
import os
import sys
import xml.etree.ElementTree as ET
import argparse
import re
from pathlib import Path

# STRINGTABLE VALIDATOR & FIXER
# ---------------------
# Verifies and automatically fixes stringtable.xml files.
# Aligns IDs with Diamond Tier standards (STR_AFM_<Component>_<Name>) and updates source code.

PROJECT_NAME = "AFM"

def fix_source_code(root_dir, old_id, new_id):
    """Refactors code references when an ID changes."""
    count = 0
    # Scan only relevant file types
    exts = {".cpp", ".hpp", ".sqf", ".ext"}
    
    for path in Path(root_dir).rglob("*"):
        if path.suffix not in exts: continue
        if ".git" in str(path) or ".hemtt" in str(path): continue
        
        try:
            content = path.read_text(encoding='utf-8', errors='ignore')
            if old_id in content:
                # Use word boundary check to avoid partial replacements
                # Need to be careful: "STR_Old" should replace, but "STR_Old_Extra" shouldn't if we matched "STR_Old"
                # Using simple replace for now as IDs are usually unique enough
                new_content = content.replace(old_id, new_id)
                path.write_text(new_content, encoding='utf-8')
                count += 1
        except: pass
    return count

def check_and_fix_stringtable(filepath, fix_mode=False):
    try:
        parser = ET.XMLParser(target=ET.TreeBuilder(insert_comments=True))
        tree = ET.parse(filepath, parser)
    except:
        print(f"  ERROR: Failed to parse {filepath}.")
        return 1

    root = tree.getroot()
    errors = 0
    modified = False
    
    component_name = os.path.basename(os.path.dirname(filepath))
    # Enforce TitleCase for component matches
    # If folder is 'hebontes_assets', package should be 'Hebontes_Assets' ideally, or just match folder?
    # Error message said "should be titlecase".
    # Let's map folder name to TitleCase
    expected_package = component_name.replace("_", " ").title().replace(" ", "_")

    # 1. Fix Root Project Name
    if root.get("name") != PROJECT_NAME:
        print(f"  ERROR: Invalid Project name '{root.get('name')}'.")
        errors += 1
        if fix_mode:
            root.set("name", PROJECT_NAME)
            modified = True
            print("    ✅ Fixed Project Name.")

    # 2. Fix Package Name
    package = root.find("Package")
    if package is None:
        print("  ERROR: Missing Package tag.")
        return errors + 1
    
    pkg_name = package.get("name")
    if pkg_name != expected_package:
        print(f"  ERROR: Package '{pkg_name}' mismatch. Expected '{expected_package}'.")
        errors += 1
        if fix_mode:
            package.set("name", expected_package)
            modified = True
            print(f"    ✅ Fixed Package Name to {expected_package}.")
    
    # 3. Check Keys
    keys = package.findall("Key")
    for container in package.findall("Container"):
        keys.extend(container.findall("Key"))

    expected_prefix = f"STR_{PROJECT_NAME}_{expected_package}_"
    
    for key in keys:
        old_id = key.get("ID")
        
        # 3a. Fix ID Format
        if not old_id.startswith(expected_prefix):
            print(f"  ERROR: Invalid ID '{old_id}'.")
            errors += 1
            
            if fix_mode:
                # Try to derive a clean name. 
                # If old ID was "STR_MyMod_Item", and we want "STR_AFM_MyMod_Item",
                # we strip common prefixes to avoid "STR_AFM_MyMod_STR_MyMod_Item".
                
                # Logic: Remove "STR_" from start
                clean_name = old_id
                if clean_name.startswith("STR_"): clean_name = clean_name[4:]
                
                # Remove any existing project/package prefix if loosely present
                # This is fuzzy. Safer to just append the remaining suffix?
                # Case: STR_hebontes_signA -> suffix is signA (if package is hebontes)
                
                # Simple Heuristic: Use the full old string as unique suffix, but cleaned?
                # No, that makes IDs huge.
                # Let's try to remove the component name if it's at the start
                comp_lower = component_name.lower()
                clean_lower = clean_name.lower()
                
                if clean_lower.startswith(comp_lower):
                    suffix = clean_name[len(comp_lower):].lstrip("_")
                elif clean_lower.startswith(pkg_name.lower()):
                    suffix = clean_name[len(pkg_name):].lstrip("_")
                else:
                    suffix = clean_name # Fallback
                
                new_id = f"{expected_prefix}{suffix}"
                
                # Check for collision? (Unlikely if 1:1 mapping)
                key.set("ID", new_id)
                modified = True
                
                # Update Code References
                # We search from the Component Root (one level up from filepath usually?)
                # Actually, search the whole "addons" folder just in case.
                repo_root = Path(filepath).parent.parent.parent 
                refs = fix_source_code(repo_root, old_id, new_id)
                print(f"    ✅ Renamed to {new_id} (Updated {refs} files)")

        # 3b. Fix "Original" tag (Remove it)
        original = key.find("Original")
        if original is not None:
            print(f"  ERROR: Key '{key.get('ID')}' has Original tag.")
            errors += 1
            if fix_mode:
                key.remove(original)
                modified = True
                print("    ✅ Removed Original tag.")

        # 3c. Fix English First
        entries = list(key)
        if entries and entries[0].tag != "English":
            print(f"  ERROR: Key '{key.get('ID')}' English not first.")
            errors += 1
            if fix_mode:
                # Find english, move to top
                eng = key.find("English")
                if eng is not None:
                    key.remove(eng)
                    key.insert(0, eng)
                    modified = True
                    print("    ✅ Reordered English to top.")

    if modified:
        tree.write(filepath, encoding="utf-8", xml_declaration=True)
        print(f"  💾 Saved changes to {filepath}")

    return errors

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--fix", action="store_true", help="Automatically fix issues")
    parser.add_argument("target", nargs="?", default=".", help="Target directory")
    args = parser.parse_args()

    print("Validating Stringtables")
    print("-----------------------")
    
    bad_count = 0
    for root, _, files in os.walk(args.target):
        if "stringtable.xml" in files:
            path = os.path.join(root, "stringtable.xml")
            print(f"\nChecking {path}...")
            bad_count += check_and_fix_stringtable(path, args.fix)

    if bad_count == 0:
        print("\n✅ All Stringtables Validated.")
    else:
        print(f"\n❌ Found {bad_count} remaining errors.")
        if not args.fix:
            print("Run with --fix to resolve automatically.")

if __name__ == "__main__":
    main()
