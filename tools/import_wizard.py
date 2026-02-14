#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import shutil
import re
from pathlib import Path
from asset_classifier import classify_asset
from rebin_guard import check_geometry_health
from p3d_debinarizer import run_debinarizer
from path_refactor import refactor_paths

# --- CONFIGURATION ---
UNIT_PREFIX = r"z\uksfta\addons"

def sanitize_name(name):
    """Converts a name to lower_snake_case."""
    base, ext = os.path.splitext(name)
    clean = base.lower().replace(" ", "_").replace("-", "_")
    clean = re.sub(r"_+", "_", clean).strip("_")
    return clean + ext.lower()

def recursive_sanitize(directory):
    """Recursively renames all files/folders in a directory to snake_case."""
    print(f"[*] Sanitizing filesystem in: {directory}")
    for root, dirs, files in os.walk(directory, topdown=False):
        for f in files:
            old_path = Path(root) / f
            new_name = sanitize_name(f)
            new_path = Path(root) / new_name
            if old_path != new_path:
                os.rename(old_path, new_path)
        for d in dirs:
            old_path = Path(root) / d
            new_name = sanitize_name(d)
            new_path = Path(root) / new_name
            if old_path != new_path:
                os.rename(old_path, new_path)

def refactor_rvmats(directory, old_prefix, new_prefix):
    """Bulk replaces paths inside .rvmat files."""
    print(f"[*] Refactoring RVMAT paths: {old_prefix} -> {new_prefix}")
    for root, _, files in os.walk(directory):
        for f in files:
            if f.endswith(".rvmat"):
                path = Path(root) / f
                content = path.read_text(errors='ignore')
                if old_prefix in content:
                    new_content = content.replace(old_prefix, new_prefix)
                    path.write_text(new_content)
                    print(f"  ✅ Patched: {f}")

def generate_config_boilerplate(directory, addon_name):
    """Generates a config.cpp based on classified assets."""
    print(f"[*] Generating config.cpp boilerplate for {addon_name}...")
    p3ds = list(Path(directory).rglob("*.p3d"))
    
    cfg_weapons = []
    cfg_vehicles = []
    
    for p in p3ds:
        category = classify_asset(p)
        name = p.stem.capitalize()
        vfs_path = f"{UNIT_PREFIX}\\{addon_name}\\{p.name}"
        
        if category == "Vest":
            cfg_weapons.append(f'    class UKSFTA_{addon_name}_{name}: Vest_Camo_Base {{ scope = 2; displayName = "UKSFTA {addon_name} {name}"; model = "{vfs_path}"; }};')
        elif category == "Helmet":
            cfg_weapons.append(f'    class UKSFTA_{addon_name}_{name}: H_HelmetB {{ scope = 2; displayName = "UKSFTA {addon_name} {name}"; model = "{vfs_path}"; }};')
        elif category == "Uniform":
            cfg_vehicles.append(f'    class UKSFTA_{addon_name}_{name}_Soldier: B_Soldier_F {{ scope = 2; displayName = "UKSFTA {addon_name} {name}"; model = "{vfs_path}"; }};')

    config = f"""class CfgPatches {{
    class UKSFTA_{addon_name} {{
        units[] = {{}};
        weapons[] = {{}};
        requiredVersion = 0.1;
        requiredAddons[] = {{"A3_Characters_F", "A3_Data_F"}};
    }};
}};

class CfgWeapons {{
    class Vest_Camo_Base;
    class H_HelmetB;
{"\n".join(cfg_weapons)}
}};

class CfgVehicles {{
    class B_Soldier_F;
{"\n".join(cfg_vehicles)}
}};
"""
    (Path(directory) / "config.cpp").write_text(config)

def run_wizard(input_dir, addon_name, old_prefix):
    """Main Ingestion Workflow."""
    print(f"\n🧙 [Import Wizard] Ingesting: {addon_name}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    
    # We assume we are running from project root if update was successful
    # But for UKSFTA-Tools test, let's target our test dir
    target_root = Path(__file__).parent.parent.parent / "UKSFTA-JacksKit"
    target_addons = target_root / "addons"
    target_addons.mkdir(exist_ok=True, parents=True)
    
    target_dir = target_addons / addon_name
    if target_dir.exists():
        shutil.rmtree(target_dir)
    shutil.copytree(input_dir, target_dir)
    
    # 1. Sanitize
    recursive_sanitize(target_dir)
    
    # 2. Refactor P3Ds
    new_vfs_prefix = f"{UNIT_PREFIX}\\{addon_name}"
    print(f"[*] Refactoring P3D paths: {old_prefix} -> {new_vfs_prefix}")
    run_debinarizer(target_dir, recursive=True, rename=(old_prefix, new_vfs_prefix))
    
    # 2b. Refactor Source Code (config.cpp, sqf, etc)
    refactor_paths(target_dir, old_prefix, new_vfs_prefix)
    
    # 3. Refactor RVMATS
    refactor_rvmats(target_dir, old_prefix, new_vfs_prefix)
    
    # 4. Generate Config
    generate_config_boilerplate(target_dir, addon_name)
    
    # 5. Final Guard
    print("\n[*] Running final integrity check...")
    for p in target_dir.rglob("*.p3d"):
        check_geometry_health(p)

    print(f"\n✨ Ingestion Complete: {target_dir}")
    print(f"ℹ️  Next: Verify config.cpp and run './build.sh build' to test.")

if __name__ == "__main__":
    if len(sys.argv) < 4:
        print("Usage: import_wizard.py <source_dir> <new_addon_name> <old_vfs_prefix>")
        sys.exit(1)
    
    run_wizard(sys.argv[1], sys.argv[2], sys.argv[3])
