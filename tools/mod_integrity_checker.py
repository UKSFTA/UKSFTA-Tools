#!/usr/bin/env python3

import os
import sys
import struct
import argparse
from pathlib import Path

# UKSFTA Mod Integrity Checker
# Validates built mod folders against Steam Workshop and Arma 3 standards

def check_pbo(pbo_path):
    """Basic PBO header validation."""
    if os.path.getsize(pbo_path) == 0:
        return False, "Zero-byte file"
    
    try:
        with open(pbo_path, "rb") as f:
            first_byte = f.read(1)
            # PBOs start with a null byte for the version entry (empty filename)
            if first_byte != b"\x00":
                return False, f"Invalid PBO header (expected 0x00, got {first_byte.hex()})"
    except Exception as e:
        return False, f"Read error: {e}"
    
    return True, "OK"

def check_integrity(mod_path, allow_unsigned=False):
    mod_path = Path(mod_path)
    errors = []
    warnings = []
    
    print(f"\nAudit: {mod_path}")
    
    if not mod_path.exists():
        errors.append("Mod path does not exist.")
        return errors, warnings

    # 1. Structure Check
    addons_dir = mod_path / "addons"
    if not addons_dir.exists():
        errors.append("Missing 'addons/' directory.")
    
    # 2. Filesystem Audit
    all_files = list(mod_path.rglob("*"))
    file_count = 0
    total_size = 0
    
    for f in all_files:
        if f.is_file():
            file_count += 1
            total_size += f.stat().st_size
            
            # Check for illegal characters in path
            if any(c in f.name for c in " @$#%^&*()+=[]{}|\\:;\"'<>,?"):
                warnings.append(f"Illegal characters in filename: {f.relative_to(mod_path)}")
            if not f.name.isascii():
                errors.append(f"Non-ASCII characters in filename: {f.relative_to(mod_path)}")

    total_size_mb = total_size / (1024 * 1024)
    print(f"  Files: {file_count}")
    print(f"  Size:  {total_size_mb:.2f} MB")

    if file_count > 100:
        warnings.append(f"High file count ({file_count}). Steam Workshop prefers consolidated PBOs.")

    # 3. PBO & Signing Validation
    if addons_dir.exists():
        pbos = list(addons_dir.glob("*.pbo"))
        if not pbos:
            errors.append("No PBO files found in addons/.")
        
        for pbo in pbos:
            valid, msg = check_pbo(pbo)
            if not valid:
                errors.append(f"Corrupt PBO: {pbo.name} ({msg})")
            
            # Check for corresponding sign file
            if not allow_unsigned:
                sign_file = pbo.with_suffix(".pbo.bisign")
                if not sign_file.exists():
                    warnings.append(f"Unsigned PBO: {pbo.name}")

    # Check for stray keys/signs outside addons
    for f in all_files:
        if f.is_file():
            if f.suffix == ".bikey" and f.parent.name != "keys" and f.parent != mod_path:
                warnings.append(f"Stray bikey found in non-standard location: {f.relative_to(mod_path)}")
            if f.suffix == ".bisign" and "addons" not in f.parts:
                errors.append(f"Stray bisign found outside addons folder: {f.relative_to(mod_path)}")
            if f.suffix == ".pbo" and "addons" not in f.parts:
                errors.append(f"Stray PBO found outside addons folder: {f.relative_to(mod_path)}")

    # 4. Leak Detection (Source files in release)
    # We ignore standard metadata and common non-binary assets if they are likely intended
    leaked_extensions = [".sqf", ".paa", ".wav", ".ogg", ".png", ".jpg", ".hpp", ".cpp"]
    for f in all_files:
        if f.is_file() and f.suffix.lower() in leaked_extensions:
            # Metadata files are allowed in root
            if f.name.lower() in ["mod.cpp", "meta.cpp"]:
                continue
            # If it's not in the addons folder, it's a potential source leak
            if "addons" not in f.parts and "keys" not in f.parts:
                warnings.append(f"Potential source leak (loose file): {f.relative_to(mod_path)}")

    # 5. Metadata Check
    mod_cpp = mod_path / "mod.cpp"
    if mod_cpp.exists():
        content = mod_cpp.read_text(errors="ignore")
        required_vars = ["name", "author", "logo"]
        for var in required_vars:
            if f"{var} =" not in content.lower() and f"{var}=" not in content.lower():
                warnings.append(f"mod.cpp might be missing '{var}' definition.")
    else:
        warnings.append("Missing mod.cpp (required for Launcher visibility).")

    meta_cpp = mod_path / "meta.cpp"
    if meta_cpp.exists():
        content = meta_cpp.read_text(errors="ignore")
        if "publishedid" not in content.lower():
            warnings.append("meta.cpp missing 'publishedid' field.")
    else:
        warnings.append("meta.cpp missing (Steam will generate this, but it's better to provide it).")

    return errors, warnings

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Mod Integrity Checker")
    parser.add_argument("path", help="Path to the built mod folder (e.g. .hemttout/release)")
    parser.add_argument("--unsigned", action="store_true", help="Allow unsigned PBOs (unit standard)")
    args = parser.parse_args()

    errors, warnings = check_integrity(args.path, allow_unsigned=args.unsigned)

    if errors:
        print("\n[!] INTEGRITY ERRORS FOUND:")
        for err in errors:
            print(f"  - {err}")
    
    if warnings:
        print("\n[?] COMPLIANCE WARNINGS:")
        for warn in warnings:
            print(f"  - {warn}")

    if not errors and not warnings:
        print("\n[OK] Mod structure is valid and clean.")
    
    if errors:
        sys.exit(1)

if __name__ == "__main__":
    main()
