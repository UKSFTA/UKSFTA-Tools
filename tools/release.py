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

try:
    from rich.console import Console
    from rich.panel import Panel
    from rich import print as rprint
    HAS_RICH = True
except ImportError:
    HAS_RICH = False
    rprint = print

# --- CONFIGURATION ---
STEAM_API_URL = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"

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

def get_workshop_details(published_ids):
    if not published_ids: return []
    details = []
    id_list = list(published_ids)
    for i in range(0, len(id_list), 100):
        chunk = id_list[i:i + 100]
        data = {"itemcount": len(chunk)}
        for j, pid in enumerate(chunk): data[f"publishedfileids[{j}]"] = pid
        try:
            encoded_data = urllib.parse.urlencode(data).encode('utf-8')
            req = urllib.request.Request(STEAM_API_URL, data=encoded_data, method='POST')
            with urllib.request.urlopen(req, timeout=15) as response:
                res = json.loads(response.read().decode('utf-8'))
                details.extend(res.get("response", {}).get("publishedfiledetails", []))
        except Exception as e:
            print(f"  ⚠️  Steam API Error: {e}")
    return details

def scrape_required_items(published_id):
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    req_ids = set()
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as response:
            html_content = response.read().decode('utf-8')
            matches = re.findall(r'class="requiredItem".*?id=(\d+)', html_content, re.DOTALL)
            for m in matches: req_ids.add(m)
    except: pass
    return req_ids

def get_mod_categories():
    included = []
    all_mentioned_ids = set()
    sources_path = os.path.join(PROJECT_ROOT, "mod_sources.txt")
    lock_path = os.path.join(PROJECT_ROOT, "mods.lock")
    lock_data = {}
    if os.path.exists(lock_path):
        try:
            with open(lock_path, "r") as f: lock_data = json.load(f).get("mods", {})
        except: pass
    if not os.path.exists(sources_path): return included, all_mentioned_ids
    is_ignore_section = False
    with open(sources_path, "r") as f:
        for line in f:
            clean_line = line.strip()
            if not clean_line or clean_line.startswith("#"):
                # Still track IDs in comments to avoid transitive duplicates
                all_mentioned_ids.update(re.findall(r"(\d{8,})", clean_line))
                continue
            if "[ignore]" in clean_line.lower():
                is_ignore_section = True
                continue
            m = re.search(r"(\d{8,})", clean_line)
            if m:
                mid = m.group(1)
                all_mentioned_ids.add(mid)
                if not is_ignore_section:
                    name = f"Mod {mid}"
                    if "#" in clean_line: name = clean_line.split("#", 1)[1].strip()
                    elif mid in lock_data: name = lock_data[mid].get("name", name)
                    if "|" in name:
                        parts = name.split("|")
                        name = f"{parts[1].strip()} ({parts[0].strip()})"
                    included.append({"id": mid, "name": name})
    return included, all_mentioned_ids

def generate_content_list(included_mods):
    if not included_mods:
        pbos = glob.glob(os.path.join(STAGING_DIR, "addons", "*.pbo"))
        if not pbos: return "No components found."
        list_str = "\n[b]Included Components:[/b]\n"
        for p in sorted(pbos): list_str += f" • {os.path.basename(p)}\n"
        return list_str
    list_str = ""
    for mod in included_mods:
        list_str += f" • [url=https://steamcommunity.com/sharedfiles/filedetails/?id={mod['id']}]{mod['name']}[/url]\n"
    return list_str

def create_vdf(app_id, workshop_id, content_path, changelog):
    desc = ""
    desc_tmpl = os.path.join(PROJECT_ROOT, "workshop_description.txt")
    if os.path.exists(desc_tmpl):
        with open(desc_tmpl, "r") as f: desc = f.read()
    
    included, all_mentioned_ids = get_mod_categories()
    
    # Discovery: Transitive Dependencies
    print(f"🔍 Analyzing transitive dependencies for {len(included)} included mods...")
    transitive_ids = set()
    ignored_app_ids = {"107410", "228800"} # Arma 3, Arma 3 Tools
    for mod in included:
        found = scrape_required_items(mod['id'])
        for fid in found:
            if fid not in all_mentioned_ids and fid not in ignored_app_ids:
                transitive_ids.add(fid)
    
    required_entries = []
    if transitive_ids:
        print(f"📦 Found {len(transitive_ids)} new transitive dependencies. Resolving names...")
        trans_details = get_workshop_details(list(transitive_ids))
        for td in trans_details:
            tid = td.get("publishedfileid")
            tname = td.get("title", f"Mod {tid}")
            required_entries.append({"id": tid, "name": f"{tname} (Transitive Dependency)"})

    # 1. Included Content
    content_list = generate_content_list(included)
    desc = desc.replace("{{INCLUDED_CONTENT}}", content_list)
    
    # 2. Required Dependencies (ONLY transitive/missing ones)
    if required_entries:
        dep_text = "\n[b]Required Mod Dependencies:[/b]\n"
        for mod in sorted(required_entries, key=lambda x: x['name']):
            dep_text += f" • [url=https://steamcommunity.com/sharedfiles/filedetails/?id={mod['id']}]{mod['name']}[/url]\n"
    else: dep_text = "None."
    
    desc = desc.replace("{{MOD_DEPENDENCIES}}", dep_text)
    
    # VDF Generation
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
    parser.add_argument("--offline", action="store_true")
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

    print(f"Running Robust Release Build (v{new_v})...")
    subprocess.run(["bash", "build.sh", "release"], check=True)

    wm_id = "0"
    if os.path.exists(PROJECT_TOML):
        with open(PROJECT_TOML, "r") as f:
            for line in f:
                if "workshop_id =" in line: wm_id = line.split("=")[1].strip().strip('"')

    if (not wm_id or wm_id == "0") and not args.dry_run and not args.offline:
        wm_id = input("Enter Workshop ID to update: ").strip()

    vdf_p, desc_p = create_vdf("107410", wm_id, STAGING_DIR, "Release v" + new_v)
    
    if args.offline:
        print(f"\n[OFFLINE] Final Workshop description generated at: {desc_p}")
        return
    if args.dry_run:
        print(f"\n[DRY-RUN] VDF generated at: {vdf_p}")
        return

    print("\n--- Steam Workshop Upload ---")
    username = os.getenv("STEAM_USERNAME") or input("Steam Username: ").strip()
    subprocess.run(["steamcmd", "+login", username, "+workshop_build_item", vdf_p, "validate", "+quit"], check=True)
    print("\n✅ SUCCESS: Mod updated on Workshop.")
    tag_name = f"v{new_v}"
    subprocess.run(["git", "tag", "-s", tag_name, "-m", f"Release {new_v}", "-f"], check=True)
    subprocess.run(["git", "push", "origin", "main", "--tags", "-f"], check=False)

if __name__ == "__main__":
    main()
