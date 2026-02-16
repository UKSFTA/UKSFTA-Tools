#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import shutil
import re
from pathlib import Path
from asset_classifier import classify_asset
# rebin_guard and others might not be needed for direct ingestion but we'll keep the imports if they exist
try:
    from p3d_debinarizer import run_debinarizer
    from path_refactor import refactor_paths
except ImportError:
    run_debinarizer = lambda *args, **kwargs: None
    refactor_paths = lambda *args, **kwargs: None

# --- CONFIGURATION ---
UNIT_PREFIX = r"z\uksfta\addons"

def get_current_project_root():
    """Attempts to find the root of the UKSFTA project we are in."""
    cwd = Path.cwd()
    current = cwd
    while current != current.parent:
        if (current / ".hemtt" / "project.toml").exists() or (current / "mod_sources.txt").exists():
            return current
        current = current.parent
    return None

def get_vfs_project_name(project_root):
    """Attempts to find the HEMTT project prefix."""
    project_toml = project_root / ".hemtt" / "project.toml"
    if project_toml.exists():
        content = project_toml.read_text()
        match = re.search(r'prefix = "([^"]+)"', content)
        if match: return match.group(1)
    name = project_root.name.lower()
    if name.startswith("uksfta-"): name = name.replace("uksfta-", "")
    return name.replace("-", "_")

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
                os.rename(old_path, new_name) # Error fixed: os.rename takes old, new
        for d in dirs:
            old_path = Path(root) / d
            new_name = sanitize_name(d)
            new_path = Path(root) / new_name
            if old_path != new_path:
                os.rename(old_path, new_path)

def generate_config_boilerplate(directory, addon_name, vfs_project):
    """Generates a config.cpp with clean inheritance and correct paths."""
    print(f"[*] Generating config.cpp boilerplate for {addon_name}...")
    p3ds = list(Path(directory).rglob("*.p3d"))
    
    v_weapons = [
        f'    class UKSFTA_{vfs_project}_{addon_name}_Vest_Base: Vest_Camo_Base {{ scope = 0; displayName = "UKSFTA {addon_name} Base"; }};',
        f'    class UKSFTA_{vfs_project}_{addon_name}_Helmet_Base: H_HelmetB {{ scope = 0; displayName = "UKSFTA {addon_name} Base"; }};'
    ]
    v_vehicles = [
        f'    class UKSFTA_{vfs_project}_{addon_name}_Uniform_Base: B_Soldier_F {{ scope = 0; displayName = "UKSFTA {addon_name} Base"; }};'
    ]

    for p in p3ds:
        category = classify_asset(p)
        name = p.stem.capitalize()
        # Ensure we use the correct VFS standard z\uksfta\addons\<project>\<addon>
        vfs_path = f"z\\uksfta\\addons\\{vfs_project}\\{addon_name}\\models\\{p.name}"
        
        if category == "Vest":
            v_weapons.append(f'    class UKSFTA_{vfs_project}_{name}: UKSFTA_{vfs_project}_{addon_name}_Vest_Base {{ scope = 2; displayName = "UKSFTA {addon_name} {name}"; model = "{vfs_path}"; }};')
        elif category == "Helmet":
            v_weapons.append(f'    class UKSFTA_{vfs_project}_{name}: UKSFTA_{vfs_project}_{addon_name}_Helmet_Base {{ scope = 2; displayName = "UKSFTA {addon_name} {name}"; model = "{vfs_path}"; }};')
        elif category == "Uniform":
            v_vehicles.append(f'    class UKSFTA_{vfs_project}_{name}_Soldier: UKSFTA_{vfs_project}_{addon_name}_Uniform_Base {{ scope = 2; displayName = "UKSFTA {addon_name} {name}"; model = "{vfs_path}"; }};')

    config = f"""class CfgPatches {{
    class UKSFTA_{vfs_project}_{addon_name} {{
        units[] = {{}};
        weapons[] = {{}};
        requiredVersion = 0.1;
        requiredAddons[] = {{"A3_Characters_F", "A3_Data_F", "UKSFTA_Mods_Main"}};
        author = "UKSF Taskforce Alpha Team";
    }};
}};

class CfgWeapons {{
    class Vest_Camo_Base;
    class H_HelmetB;
{"\n".join(v_weapons)}
}};

class CfgVehicles {{
    class B_Soldier_F;
{"\n".join(v_vehicles)}
}};
"""
    (Path(directory) / "config.cpp").write_text(config.strip() + "\n")

def run_wizard(input_dir, addon_name, old_prefix):
    """Main Ingestion Workflow."""
    print(f"\n🧙 [Import Wizard] Ingesting: {addon_name}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    
    project_root = get_current_project_root()
    if not project_root:
        print("❌ Error: Not inside a UKSFTA project directory.")
        sys.exit(1)
        
    target_addons = project_root / "addons"
    target_addons.mkdir(exist_ok=True, parents=True)
    
    target_dir = target_addons / addon_name
    if target_dir.exists(): shutil.rmtree(target_dir)
    shutil.copytree(input_dir, target_dir)
    
    recursive_sanitize(target_dir)
    vfs_project = get_vfs_project_name(project_root)
    # Align PBOPREFIX to Diamond Tier standard
    vfs_prefix = f"z\\uksfta\\addons\\{vfs_project}\\{addon_name}"
    (target_dir / "$PBOPREFIX$").write_text(vfs_prefix + "\n")
    
    run_debinarizer(target_dir, recursive=True, rename=(old_prefix, vfs_prefix))
    refactor_paths(target_dir, old_prefix, vfs_prefix)
    generate_config_boilerplate(target_dir, addon_name, vfs_project)
    
    print(f"\n✨ Ingestion Complete: {target_dir}")

if __name__ == "__main__":
    if len(sys.argv) < 4:
        print("Usage: import_wizard.py <source_dir> <new_addon_name> <old_vfs_prefix>"); sys.exit(1)
    run_wizard(sys.argv[1], sys.argv[2], sys.argv[3])
