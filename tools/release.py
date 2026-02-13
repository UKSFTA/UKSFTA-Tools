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
    all_acknowledged = set()
    
    if not os.path.exists(MOD_SOURCES_FILE): return included, all_acknowledged

    is_ignore_section = False
    with open(MOD_SOURCES_FILE, "r") as f:
        content = f.read()
        all_acknowledged.update(re.findall(r"(\d{8,})", content))
        
        # Reset and parse lines for logical categorization
        f.seek(0)
        for line in f:
            clean_line = line.strip()
            if not clean_line or clean_line.startswith("#"): continue
            if "[ignore]" in clean_line.lower(): is_ignore_section = True; continue
            m = re.search(r"(\d{8,})", clean_line)
            if m and not is_ignore_section:
                mid = m.group(1); name = f"Mod {mid}"
                if "#" in clean_line: name = clean_line.split("#", 1)[1].strip()
                if "|" in name: parts = name.split("|"); name = f"{parts[1].strip()} ({parts[0].strip()})"
                included.append({"id": mid, "name": name})
    return included, all_acknowledged

def create_vdf(app_id, workshop_id, content_path, changelog):
    desc = ""
    tmpl_path = os.path.join(PROJECT_ROOT, "workshop_description.txt")
    if os.path.exists(tmpl_path):
        with open(tmpl_path, "r") as f: desc = f.read()
    
    included, all_acknowledged = get_mod_categories()
    
    # Discovery: Transitive Dependencies
    print(f"🔍 Analyzing dependencies for {len(included)} included mods...")
    inc_ids = [m['id'] for m in included]
    resolved = resolve_transitive_dependencies(inc_ids, all_acknowledged)
    
    required_entries = []
    for mid, meta in resolved.items():
        if mid not in inc_ids:
            required_entries.append({"id": mid, "name": f"{meta['name']} (Transitive Dependency)"})

    # 1. Included Content
    content_list = ""
    if included:
        for mod in included: content_list += f" • [url=https://steamcommunity.com/sharedfiles/filedetails/?id={mod['id']}]{mod['name']}[/url]\n"
    else:
        pbos = glob.glob(os.path.join(STAGING_DIR, "addons", "*.pbo"))
        if not pbos: content_list = "No components found."
        else:
            content_list = "\n[b]Included Components:[/b]\n"
            for p in sorted(pbos): content_list += f" • {os.path.basename(p)}\n"
    desc = desc.replace("{{INCLUDED_CONTENT}}", content_list)
    
    # 2. Required Dependencies
    if required_entries:
        dep_text = "\n[b]Required Mod Dependencies:[/b]\n"
        for mod in sorted(required_entries, key=lambda x: x['name']):
            dep_text += f" • [url=https://steamcommunity.com/sharedfiles/filedetails/?id={mod['id']}]{mod['name']}[/url]\n"
    else: dep_text = "None."
    desc = desc.replace("{{MOD_DEPENDENCIES}}", dep_text)
    
    # VDF Logic
    config = {"id": "0", "tags": ["Mod", "Addon"]}
    if os.path.exists(PROJECT_TOML):
        with open(PROJECT_TOML, "r") as f:
            for line in f:
                if "workshop_id =" in line: config["id"] = line.split("=")[1].strip().strip('"')
                if "workshop_tags =" in line:
                    m = re.search(r"\[(.*?)\]", line)
                    if m: config["tags"] = [t.strip().strip('"').strip("'") for t in m.group(1).split(",")]
    
    tags_vdf = "".join([f'        "{i}" "{t}"\n' for i, t in enumerate(config["tags"])])
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

    wm_id = "0"
    if os.path.exists(PROJECT_TOML):
        with open(PROJECT_TOML, "r") as f:
            for line in f:
                if "workshop_id =" in line: wm_id = line.split("=")[1].strip().strip('"')

    if (not wm_id or wm_id == "0") and not args.dry_run and not args.offline:
        wm_id = input("Enter Workshop ID: ").strip()

    vdf_p, desc_p = create_vdf("107410", wm_id, STAGING_DIR, "Release v" + new_v)
    
    if args.offline:
        print(f"\n[OFFLINE] Description: {desc_p}\n[OFFLINE] VDF: {vdf_p}")
        return
    if args.dry_run:
        print(f"\n[DRY-RUN] VDF: {vdf_p}")
        return

    username = os.getenv("STEAM_USERNAME") or input("Steam Username: ").strip()
    subprocess.run(["steamcmd", "+login", username, "+workshop_build_item", vdf_p, "validate", "+quit"], check=True)
    print("\n✅ Mod updated on Workshop.")
    tag_name = f"v{new_v}"
    subprocess.run(["git", "tag", "-s", tag_name, "-m", f"Release {new_v}", "-f"], check=True)
    subprocess.run(["git", "push", "origin", "main", "--tags", "-f"], check=False)

if __name__ == "__main__": main()
