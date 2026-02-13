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
import html
import argparse
import multiprocessing
import time

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
LOCK_FILE = "mods.lock"

def find_version_file():
    addons_dir = os.path.join(PROJECT_ROOT, "addons")
    if not os.path.exists(addons_dir): return None
    for root, _, files in os.walk(addons_dir):
        if "script_version.hpp" in files: return os.path.join(root, "script_version.hpp")
    return None

VERSION_FILE = find_version_file()

def print_steam_info():
    info = """
[bold cyan]How Steam Workshop Uploads Work:[/bold cyan]
1. [white]Raw Files:[/] SteamCMD does [bold red]NOT[/bold red] upload a ZIP file.
2. [white]Content Sync:[/] It uploads the [bold green]raw folder structure[/] (addons, keys, etc).
3. [white]Differential:[/] Steam compares your local files with the server and only uploads the changes.
4. [white]VDF Config:[/] The [dim]upload.vdf[/] tells Steam which folder to scan and what metadata to set.
    """
    if HAS_RICH:
        rprint(Panel(info, title="SteamCMD Technical Context", border_style="cyan"))
    else:
        print("\n--- SteamCMD Technical Context ---")

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
    required = []
    sources_path = os.path.join(PROJECT_ROOT, "mod_sources.txt")
    lock_path = os.path.join(PROJECT_ROOT, "mods.lock")
    lock_data = {}
    if os.path.exists(lock_path):
        try:
            with open(lock_path, "r") as f: lock_data = json.load(f).get("mods", {})
        except: pass
    if not os.path.exists(sources_path): return included, required
    is_ignore_section = False
    with open(sources_path, "r") as f:
        for line in f:
            clean_line = line.strip()
            if not clean_line or clean_line.startswith("#"): continue
            if "[ignore]" in clean_line.lower():
                is_ignore_section = True
                continue
            m = re.search(r"(\d{8,})", clean_line)
            if m:
                mid = m.group(1); name = f"Mod {mid}"
                if "#" in clean_line: name = clean_line.split("#", 1)[1].strip()
                elif mid in lock_data: name = lock_data[mid].get("name", name)
                if "|" in name:
                    parts = name.split("|")
                    name = f"{parts[1].strip()} ({parts[0].strip()})"
                entry = {"id": mid, "name": name}
                if is_ignore_section: required.append(entry)
                else: included.append(entry)
    return included, required

def generate_content_list(included_mods):
    if not included_mods:
        pbos = glob.glob(os.path.join(STAGING_DIR, "addons", "*.pbo"))
        if not pbos: return "No components found."
        list_str = "\n[b]Included Components:[/b]\n"
        for p in sorted(pbos): list_str += f" • {os.path.basename(p)}\n"
        return list_str
    list_str = ""
    for mod in included_mods: list_str += f" • {mod['name']}\n"
    return list_str

def get_workshop_config():
    config = {"id": "0", "tags": ["Mod", "Addon"]}
    if os.path.exists(PROJECT_TOML):
        with open(PROJECT_TOML, "r") as f:
            for line in f:
                if "workshop_id =" in line: config["id"] = line.split("=")[1].strip().strip('"')
                if "workshop_tags =" in line:
                    m = re.search(r"\[(.*?)\]", line)
                    if m: config["tags"] = [t.strip().strip('"').strip("'") for t in m.group(1).split(",")]
    return config

def create_vdf(app_id, workshop_id, content_path, changelog):
    desc = ""
    desc_tmpl = os.path.join(PROJECT_ROOT, "workshop_description.txt")
    if os.path.exists(desc_tmpl):
        with open(desc_tmpl, "r") as f: desc = f.read()
    included, required = get_mod_categories()
    content_list = generate_content_list(included)
    desc = desc.replace("{{INCLUDED_CONTENT}}", content_list)
    if required:
        dep_text = "\n[b]Required Mod Dependencies:[/b]\n"
        for mod in sorted(required, key=lambda x: x['name']):
            dep_text += f" • [url=https://steamcommunity.com/sharedfiles/filedetails/?id={mod['id']}]{mod['name']}[/url]\n"
    else: dep_text = "None."
    desc = desc.replace("{{MOD_DEPENDENCIES}}", dep_text)
    config = get_workshop_config()
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
    with open(vdf_path, "w") as f: f.write(vdf.strip())
    if not os.getenv("PYTEST_CURRENT_TEST"):
        desc_out = os.path.join(PROJECT_ROOT, "workshop_description_final.txt")
        with open(desc_out, "w") as f: f.write(desc)
    else: desc_out = "workshop_description_final.txt"
    return vdf_path, desc_out

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Release Tool")
    parser.add_argument("-p", "--patch", action="store_true")
    parser.add_argument("-m", "--minor", action="store_true")
    parser.add_argument("-M", "--major", action="store_true")
    parser.add_argument("-n", "--none", action="store_true")
    parser.add_argument("-y", "--yes", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--offline", action="store_true", help="Generate description/VDF but skip Steam upload")
    args = parser.parse_args()
    print_steam_info()
    cur_v, _ = get_current_version()
    print(f"Current version: {cur_v}")
    choice = 'n'
    if args.patch: choice = 'p'
    elif args.minor: choice = 'm'
    elif args.major: choice = 'major'
    elif args.none: choice = 'n'
    elif not args.yes: choice = input("Bump version? [p]atch/[m]inor/[M]ajor/[n]one: ").lower()
    new_v = cur_v
    if choice in ['p', 'm', 'major']:
        part = "patch"
        if choice == 'm': part = "minor"
        if choice == 'major': part = "major"
        new_v = bump_version(part)
        if not args.dry_run:
            subprocess.run(["git", "add", VERSION_FILE], check=True)
            subprocess.run(["git", "commit", "-S", "-m", f"chore: bump version to {new_v}"], check=True)
    print(f"Running Robust Release Build (v{new_v})...")
    subprocess.run(["bash", "build.sh", "release"], check=True)
    ws_config = get_workshop_config()
    workshop_id = ws_config["id"]
    if (not workshop_id or workshop_id == "0") and not args.dry_run and not args.offline:
        workshop_id = input("Enter Workshop ID to update: ").strip()
    vdf_path, desc_path = create_vdf("107410", workshop_id, STAGING_DIR, "Release v" + new_v)
    if args.offline:
        print(f"\n[OFFLINE] Final Workshop description generated at: {desc_path}")
        print("[OFFLINE] VDF generated at: " + vdf_path)
        return
    if args.dry_run:
        print(f"\n[DRY-RUN] VDF generated at: {vdf_path}")
        return
    print("\n--- Steam Workshop Upload ---")
    username = os.getenv("STEAM_USERNAME") or input("Steam Username: ").strip()
    cmd = ["steamcmd", "+login", username, "+workshop_build_item", vdf_path, "validate", "+quit"]
    try:
        subprocess.run(cmd, check=True)
        print("\n✅ SUCCESS: Mod updated on Workshop.")
        tag_name = f"v{new_v}"
        subprocess.run(["git", "tag", "-s", tag_name, "-m", f"Release {new_v}", "-f"], check=True)
        subprocess.run(["git", "push", "origin", "main", "--tags", "-f"], check=False)
    except Exception as e:
        print(f"Error: {e}"); sys.exit(1)

if __name__ == "__main__": main()
