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

def get_mod_categories():
    included = []
    ignored = set()
    all_ack = set()
    if not os.path.exists(MOD_SOURCES_FILE): return included, ignored, all_ack
    is_ignore_section = False
    with open(MOD_SOURCES_FILE, "r") as f:
        f_content = f.read(); all_ack.update(re.findall(r"(\d{8,})", f_content))
        f.seek(0)
        for line in f:
            cl = line.strip()
            if not cl or cl.startswith("#"): continue
            if "[ignore]" in cl.lower(): is_ignore_section = True; continue
            m = re.search(r"(\d{8,})", cl)
            if m:
                mid = m.group(1)
                if is_ignore_section: ignored.add(mid)
                else:
                    name = f"Mod {mid}"
                    if "#" in cl: name = cl.split("#", 1)[1].strip()
                    if "|" in name: parts = name.split("|"); name = f"{parts[1].strip()} ({parts[0].strip()})"
                    included.append({"id": mid, "name": name})
    return included, ignored, all_ack

def get_automatic_tags():
    """Derives Workshop tags based on project name and metadata."""
    tags = set(["Mod", "Addon", "Multiplayer", "Coop", "Realism", "Modern"])
    project_name = os.path.basename(PROJECT_ROOT).lower()
    
    if "map" in project_name or "terrain" in project_name:
        tags.add("Map")
    if "script" in project_name:
        tags.add("Tools")
    if "temp" in project_name or "mods" in project_name:
        tags.add("Other")
        
    return list(tags)

def get_workshop_config():
    config = {"id": "0", "tags": get_automatic_tags()}
    if os.path.exists(PROJECT_TOML):
        with open(PROJECT_TOML, "r") as f:
            for line in f:
                if "workshop_id =" in line: config["id"] = line.split("=")[1].strip().strip('"')
                if "workshop_tags =" in line:
                    m = re.search(r"\[(.*?)\]", line)
                    if m:
                        # Combine manual tags with automatic ones
                        manual = [t.strip().strip('"').strip("'") for t in m.group(1).split(",") if t.strip()]
                        config["tags"] = list(set(config["tags"] + manual))
    return config

def create_vdf(app_id, workshop_id, content_path, changelog):
    desc = ""
    tmpl_path = os.path.join(PROJECT_ROOT, "workshop_description.txt")
    if os.path.exists(tmpl_path):
        with open(tmpl_path, "r") as f: desc = f.read()
    
    included, ignored, all_ack = get_mod_categories()
    
    # Discovery: Transitive Repack check
    print(f"🔍 Analyzing dependencies for {len(included)} bundled mods...")
    inc_ids = [m['id'] for m in included]
    resolved = resolve_transitive_dependencies(inc_ids, all_ack)
    
    transitive_requirements = []
    for mid, meta in resolved.items():
        if mid not in inc_ids and mid not in ignored:
            transitive_requirements.append({"id": mid, "name": f"{meta['name']} (Included)"})

    # 1. Included Content (Directly listed mods from mod_sources.txt)
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
    
    # 2. Requirements (Transitive dependencies bundled in the repack)
    if transitive_requirements:
        dep_text = "[b]Repacked Dependencies:[/b]\n"
        dep_text += "[i]The following items are already included in this modpack and do not require separate subscription:[/i]\n[list]\n"
        for mod in sorted(transitive_requirements, key=lambda x: x['name']):
            dep_text += f" [*] {mod['name']} (Workshop ID: {mod['id']})\n"
        dep_text += "[/list]\n"
    else: dep_text = "None. (All core requirements handled by unit launcher)"
    desc = desc.replace("{{MOD_DEPENDENCIES}}", dep_text)
    
    # Workshop Config (ID & Tags)
    ws_config = get_workshop_config()
    tags_vdf = "".join([f'        "{i}" "{t}"\n' for i, t in enumerate(sorted(ws_config["tags"]))])
    
    vdf = f"""
"workshopitem"
{{
    "appid" "{app_id}"
    "publishedfileid" "{workshop_id}"
    "contentfolder" "{content_path}"
    "changenote" "{changelog}"
    "description" "{desc}"
    "tags"
    {{
{tags_vdf}    }}
}}
"""
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

    print(f"Running Build (v{new_v})...")
    subprocess.run(["bash", "build.sh", "release"], check=True)

    ws_config = get_workshop_config()
    workshop_id = ws_config["id"]
    if (not workshop_id or workshop_id == "0") and not args.dry_run and not args.offline:
        workshop_id = input("Enter Workshop ID: ").strip()

    vdf_p, desc_p = create_vdf("107410", workshop_id, STAGING_DIR, "Release v" + new_v)
    
    if args.offline:
        print(f"\n[OFFLINE] Description: {desc_p}\n[OFFLINE] VDF: {vdf_p}")
        return
    if args.dry_run:
        print(f"\n[DRY-RUN] VDF: {vdf_p}")
        return

    username = os.getenv("STEAM_USERNAME")
    password = os.getenv("STEAM_PASSWORD")
    if not username and not args.dry_run and not args.offline:
        username = input("Steam Username: ").strip()

    print("\n--- Steam Workshop Upload ---")
    login_args = [username]
    if password: login_args.append(password)
    
    cmd = ["steamcmd", "+login"] + login_args + ["+workshop_build_item", vdf_p, "validate", "+quit"]
    
    try:
        subprocess.run(cmd, check=True)
        print("\n✅ Mod updated on Workshop.")
        tag_name = f"v{new_v}"
        subprocess.run(["git", tag_name, "-s", "-m", f"Release {new_v}", "-f"], check=True)
        subprocess.run(["git", "push", "origin", "main", "--tags", "-f"], check=False)
    except Exception as e:
        print(f"Error: {e}"); sys.exit(1)

if __name__ == "__main__": main()
