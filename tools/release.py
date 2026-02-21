#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import subprocess
import shutil
import json
import glob
import urllib.request
import urllib.parse
from manage_mods import get_mod_categories
import html
import argparse
import multiprocessing
import time
from workshop_utils import resolve_transitive_dependencies, get_bulk_metadata

try:
    from rich.console import Console
    from rich.panel import Panel
    from rich import print as rprint
    HAS_RICH = True
except ImportError:
    HAS_RICH = False
    rprint = print

# --- CONFIGURATION ---
def resolve_project_root():
    current = os.getcwd()
    if os.path.exists(os.path.join(current, ".hemtt", "project.toml")) or os.path.exists(os.path.join(current, "mod_sources.txt")):
        return current
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

PROJECT_ROOT = resolve_project_root()
HEMTT_OUT = os.path.join(PROJECT_ROOT, ".hemttout")
STAGING_DIR = os.path.join(HEMTT_OUT, "release")
PROJECT_TOML = os.path.join(PROJECT_ROOT, ".hemtt", "project.toml")
LOCK_FILE = os.path.join(PROJECT_ROOT, "mods.lock")
MOD_SOURCES_FILE = os.path.join(PROJECT_ROOT, "mod_sources.txt")

def load_env():
    env_paths = [os.path.join(PROJECT_ROOT, ".env"), os.path.join(PROJECT_ROOT, "..", "UKSFTA-Tools", ".env")]
    for env_path in env_paths:
        if os.path.exists(env_path):
            with open(env_path, "r") as f:
                for line in f:
                    line = line.strip()
                    if line and not line.startswith("#") and "=" in line:
                        k, v = line.split("=", 1); os.environ[k.strip()] = v.strip()
            return

def find_version_file():
    addons_dir = os.path.join(PROJECT_ROOT, "addons")
    if not os.path.exists(addons_dir): return None
    for root, _, files in os.walk(addons_dir):
        if "script_version.hpp" in files: return os.path.join(root, "script_version.hpp")
    return None

VERSION_FILE = find_version_file()

def get_current_version():
    if not VERSION_FILE or not os.path.exists(VERSION_FILE): return "0.0.0", (0, 0, 0)
    with open(VERSION_FILE, "r") as f: content = f.read()
    m = re.search(r"#define\s+MAJOR\s+(\d+)", content)
    mi = re.search(r"#define\s+MINOR\s+(\d+)", content)
    p = re.search(r"#define\s+PATCHLVL\s+(\d+)", content)
    if not all([m, mi, p]): return "0.0.0", (0, 0, 0)
    return f"{m.group(1)}.{mi.group(1)}.{p.group(1)}", (int(m.group(1)), int(mi.group(1)), int(p.group(1)))

def bump_version(part="patch"):
    v_str, (ma, mi, pa) = get_current_version()
    if part == "major": ma += 1; mi = 0; pa = 0
    elif part == "minor": mi += 1; pa = 0
    else: pa += 1
    new_v = f"{ma}.{mi}.{pa}"
    with open(VERSION_FILE, "r") as f: content = f.read()
    content = re.sub(r"#define\s+MAJOR\s+\d+", f"#define MAJOR {ma}", content)
    content = re.sub(r"#define\s+MINOR\s+\d+", f"#define MINOR {mi}", content)
    content = re.sub(r"#define\s+PATCHLVL\s+\d+", f"#define PATCHLVL {pa}", content)
    with open(VERSION_FILE, "w") as f: f.write(content)
    return new_v

def get_automatic_tags():
    tags = set(["Mod", "Addon", "Multiplayer", "Coop", "Realism", "Modern"])
    p_name = os.path.basename(PROJECT_ROOT).lower()
    if "map" in p_name or "terrain" in p_name: tags.add("Map")
    if "script" in p_name: tags.add("Tools")
    if "temp" in p_name or "mods" in p_name: tags.add("Other")
    return list(tags)

def get_workshop_config():
    config = {"id": "0", "tags": get_automatic_tags()}
    if os.path.exists(PROJECT_TOML):
        with open(PROJECT_TOML, "r") as f:
            content = f.read()
            m_id = re.search(r'workshop_id = "(.*)"', content)
            if m_id: config["id"] = m_id.group(1)
            m_tags = re.search(r'workshop_tags = \[(.*?)\]', content)
            if m_tags:
                manual = [t.strip().strip('"').strip("'") for t in m_tags.group(1).split(",") if t.strip()]
                config["tags"] = list(set(config["tags"] + manual))
    return config

def create_vdf(app_id, workshop_id, content_path, changelog):
    desc = ""
    tmpl = os.path.join(PROJECT_ROOT, "workshop_description.txt")
    if os.path.exists(tmpl):
        with open(tmpl, "r") as f: desc = f.read()
    
    included, ignored, all_ack = get_mod_categories()
    inc_ids = [m['id'] for m in included]
    resolved = resolve_transitive_dependencies(inc_ids, all_ack)
    
    trans_reqs = []
    for mid, meta in resolved.items():
        if mid not in inc_ids and mid not in ignored:
            trans_reqs.append({"id": mid, "name": f"{meta['name']} (Included)"})

    content_list = ""
    if included:
        for mod in included: content_list += f" [*] {mod['name']} (Workshop ID: {mod['id']})\n"
    else:
        pbos = glob.glob(os.path.join(STAGING_DIR, "addons", "*.pbo"))
        if not pbos: content_list = " [*] No components found."
        else:
            content_list = "\n[b]Included Components:[/b]\n[list]\n"
            for p in sorted(pbos): content_list += f" [*] {os.path.basename(p)}\n"
            content_list += "[/list]\n"
    desc = desc.replace("{{INCLUDED_CONTENT}}", content_list)
    
    if trans_reqs:
        dep_text = "[b]Repacked Dependencies:[/b]\n[i]Included in this modpack:[/i]\n[list]\n"
        for mod in sorted(trans_reqs, key=lambda x: x['name']):
            dep_text += f" [*] {mod['name']} (Workshop ID: {mod['id']})\n"
        dep_text += "[/list]\n"
    else: dep_text = "None. (All core requirements handled by unit launcher)"
    desc = desc.replace("{{MOD_DEPENDENCIES}}", dep_text)
    
    ws_config = get_workshop_config()
    tags_vdf = "".join([f'        "{i}" "{t}"\n' for i, t in enumerate(sorted(ws_config["tags"]))])
    
    vdf = f'\"workshopitem\"\n{{\n    \"appid\" \"{app_id}\"\n    \"publishedfileid\" \"{workshop_id}\"\n    \"contentfolder\" \"{content_path}\"\n    \"changenote\" \"{changelog}\"\n    \"description\" \"{desc}\"\n    \"tags\"\n    {{\n{tags_vdf}    }}\n}}'
    
    vdf_path = os.path.join(HEMTT_OUT, "upload.vdf")
    os.makedirs(os.path.dirname(vdf_path), exist_ok=True)
    with open(vdf_path, "w") as f: f.write(vdf.strip())
    desc_out = os.path.join(PROJECT_ROOT, "workshop_description_final.txt")
    if not os.getenv("PYTEST_CURRENT_TEST"):
        with open(desc_out, "w") as f: f.write(desc)
    return vdf_path, desc_out

def main():
    load_env()
    parser = argparse.ArgumentParser(description="UKSFTA Release Tool")
    parser.add_argument("-p", "--patch", action="store_true", help="Bump patch")
    parser.add_argument("-m", "--minor", action="store_true", help="Bump minor")
    parser.add_argument("-M", "--major", action="store_true", help="Bump major")
    parser.add_argument("-n", "--none", action="store_true", help="No bump")
    parser.add_argument("-y", "--yes", action="store_true", help="Auto-yes")
    parser.add_argument("-t", "--threads", type=int, help="Number of threads for HEMTT (default: cpu_count - 2)")
    parser.add_argument("--skip-build", action="store_true", help="Skip the build process and use existing artifacts in .hemttout/release")
    parser.add_argument("--dry-run", action="store_true", help="Simulate release")
    parser.add_argument("--offline", action="store_true", help="Offline mode")
    args = parser.parse_args()

    v_str, _ = get_current_version()
    print(f"Current version: {v_str}")
    choice = 'n'
    if args.patch: choice = 'p'
    elif args.minor: choice = 'm'
    elif args.major: choice = 'major'
    elif args.none: choice = 'n'
    elif not args.yes: choice = input("Bump version? [p]atch/[m]inor/[M]ajor/[n]one: ").lower()

    new_v = v_str
    if choice in ['p', 'm', 'major']:
        part = "patch"
        if choice == 'm': part = "minor"
        if choice == 'major': part = "major"
        new_v = bump_version(part)
        if not args.dry_run:
            subprocess.run(["git", "add", VERSION_FILE], check=True)
            subprocess.run(["git", "commit", "-S", "-m", f"chore: bump version to {new_v}"], check=True)

    if args.skip_build:
        print("⏩ Skipping build as requested. Using existing artifacts in .hemttout/release...")
        if not os.path.exists(STAGING_DIR) or not os.listdir(STAGING_DIR):
            print(f"❌ Error: Release directory {STAGING_DIR} is empty or does not exist. Cannot skip build.")
            sys.exit(1)
        if choice in ['p', 'm', 'major']:
            print("⚠️  Warning: Version was bumped, but build was skipped. Uploaded PBOs will still have the old version!")
    else:
        print(f"Running Build (v{new_v})...")
        # Set HEMTT_TEMP_DIR and standard TEMP variables to a project-local directory for build stability
        build_env = os.environ.copy()
        build_temp_dir = os.path.join(HEMTT_OUT, "tmp")
        os.makedirs(build_temp_dir, exist_ok=True)
        build_env["HEMTT_TEMP_DIR"] = build_temp_dir
        build_env["TMPDIR"] = build_temp_dir
        build_env["TEMP"] = build_temp_dir
        build_env["TMP"] = build_temp_dir
        
        # Calculate threads: use provided count or default (cpu_count - 2)
        if args.threads:
            num_threads = str(args.threads)
        else:
            cpu_count = multiprocessing.cpu_count()
            num_threads = str(max(1, cpu_count - 2))
            
        subprocess.run(["bash", "build.sh", "release", "--threads", num_threads], check=True, env=build_env)

    ws_config = get_workshop_config()
    workshop_id = ws_config["id"]
    if (not workshop_id or workshop_id == "0") and not args.dry_run and not args.offline:
        workshop_id = input("Enter Workshop ID: ").strip()

    # Prepare the correct content path for Steam
    # We want to upload the folder structure: @ModName/addons/*.pbo
    # build.sh creates this in .hemttout/zip_staging/@PROJECT_ID
    project_id = os.path.basename(PROJECT_ROOT)
    content_staging = os.path.join(HEMTT_OUT, "zip_staging")
    
    # UKSFTA DIAMOND TIER: Flat Structure Enforcement
    # Standard Arma 3 Workshop structure requires the 'addons' folder to be in the root of the upload.
    # We point directly to the folder containing 'addons', NOT the parent @folder.
    potential_root = os.path.join(content_staging, f"@{project_id}")
    if os.path.exists(os.path.join(potential_root, "addons")):
        vdf_content_path = potential_root
    elif os.path.exists(os.path.join(STAGING_DIR, "addons")):
        vdf_content_path = STAGING_DIR
    else:
        print("❌ CRITICAL ERROR: 'addons' folder not found in any staging directory.")
        print(f"Searched: {potential_root} and {STAGING_DIR}")
        sys.exit(1)

    vdf_p, desc_p = create_vdf("107410", workshop_id, vdf_content_path, "Release v" + new_v)
    
    if args.offline:
        print(f"\n[OFFLINE] Diamond Tier Staging Complete.")
        print(f"[OFFLINE] Description: {desc_p}")
        print(f"[OFFLINE] VDF: {vdf_p} (Points to: {vdf_content_path})")
        if workshop_id == "0":
            print("⚠️  Warning: Workshop ID is '0'. This VDF will create a NEW item if used.")
        return
    if args.dry_run:
        print(f"\n[DRY-RUN] VDF: {vdf_p}")
        return

    username = os.getenv("STEAM_USERNAME")
    password = os.getenv("STEAM_PASSWORD")
    if not username and not args.dry_run and not args.offline:
        username = input("Steam Username: ").strip()

    print("\n--- Steam Workshop Upload (with validation) ---")
    login_args = [username]
    if password: login_args.append(password)
    
    # Using 'validate' flag to ensure file integrity after upload
    # We also explicitly use 'quit' to ensure the process terminates correctly.
    cmd = ["steamcmd", "+login"] + login_args + ["+workshop_build_item", vdf_p, "validate", "+quit"]
    
    try:
        # Running steamcmd with a timeout of 15 minutes to prevent hung processes, but allowing enough time for large uploads
        result = subprocess.run(cmd, check=True, timeout=900)
        print("\n✅ Mod updated and validated on Workshop.")
        tag_name = f"v{new_v}"
        subprocess.run(["git", "tag", "-s", tag_name, "-m", f"Release {new_v}"], check=True)
        subprocess.run(["git", "push", "origin", "main", "--tags", "-f"], check=False)
    except subprocess.TimeoutExpired:
        print("\n❌ Error: SteamCMD timed out after 15 minutes. The upload might still be processing, please check the Workshop.")
        sys.exit(1)
    except Exception as e:
        print(f"\n❌ Error: {e}"); sys.exit(1)

if __name__ == "__main__": main()
