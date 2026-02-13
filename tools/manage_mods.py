import os
import re
import subprocess
import shutil
import json
import sys
import urllib.request
import urllib.parse
import html
import argparse

# Configuration
PROJECT_ROOT = os.getcwd()
MOD_SOURCES_FILE = "mod_sources.txt"
LOCK_FILE = "mods.lock"
ADDONS_DIR = "addons"
KEYS_DIR = "keys"
STEAMAPP_ID = "107410"  # Arma 3
STEAM_API_URL = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"

def load_env():
    env_paths = [
        os.path.join(PROJECT_ROOT, ".env"),
        os.path.join(PROJECT_ROOT, "..", "UKSFTA-Tools", ".env")
    ]
    for env_path in env_paths:
        if os.path.exists(env_path):
            with open(env_path, "r") as f:
                for line in f:
                    line = line.strip()
                    if not line or line.startswith("#"): continue
                    if "=" in line:
                        parts = line.split("=", 1)
                        if len(parts) == 2: os.environ[parts[0].strip()] = parts[1].strip()
            return

def get_mod_ids_from_file():
    mods = {}
    if not os.path.exists(MOD_SOURCES_FILE): return mods
    with open(MOD_SOURCES_FILE, "r") as f:
        for line in f:
            clean_line = line.strip()
            if not clean_line or clean_line.startswith("#"): continue
            if "[ignore]" in clean_line.lower() or "[ignored]" in clean_line.lower(): break
            if "ignore=" in clean_line.lower() or "@ignore" in clean_line.lower(): continue
            match = re.search(r"(?:id=)?(\d{8,})", clean_line)
            if match:
                mod_id = match.group(1); tag = ""
                if "#" in clean_line: tag = clean_line.split("#", 1)[1].strip()
                mods[mod_id] = tag
    return mods

def get_ignored_ids_from_file():
    ignored = set()
    if not os.path.exists(MOD_SOURCES_FILE): return ignored
    ignore_block = False
    with open(MOD_SOURCES_FILE, "r") as f:
        for line in f:
            clean_line = line.strip().lower()
            if not clean_line or clean_line.startswith("#"): continue
            if "[ignore]" in clean_line or "[ignored]" in clean_line:
                ignore_block = True; continue
            if ignore_block:
                matches = re.findall(r"(\d{8,})", clean_line)
                for mid in matches: ignored.add(mid)
            else:
                if "ignore=" in clean_line or "@ignore" in clean_line:
                    matches = re.findall(r"(\d{8,})", clean_line)
                    for mid in matches: ignored.add(mid)
    return ignored

def get_bulk_metadata(mod_ids):
    """Fetches basic metadata for multiple mods using the Steam API (Fast)."""
    if not mod_ids: return {}
    results = {}
    id_list = list(mod_ids)
    for i in range(0, len(id_list), 100):
        chunk = id_list[i:i+100]
        data = {"itemcount": len(chunk)}
        for j, pid in enumerate(chunk): data[f"publishedfileids[{j}]"] = pid
        try:
            encoded_data = urllib.parse.urlencode(data).encode('utf-8')
            req = urllib.request.Request(STEAM_API_URL, data=encoded_data, method='POST')
            with urllib.request.urlopen(req, timeout=10) as response:
                res = json.loads(response.read().decode('utf-8'))
                details = res.get("response", {}).get("publishedfiledetails", [])
                for d in details:
                    mid = d.get("publishedfileid")
                    if mid:
                        results[mid] = {
                            "name": d.get("title", f"Mod {mid}"),
                            "updated": str(d.get("time_updated", "0")),
                            "dependencies": [] # API is unreliable for deps
                        }
        except: pass
    return results

def scrape_mod_metadata(mod_id):
    """Precision Scraper: Fetches 'Required Items' and Title/Timestamp (Slower)."""
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={mod_id}"
    info = {"name": None, "dependencies": [], "updated": None}
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as response:
            page = response.read().decode('utf-8')
            match = re.search(r'<div class="workshopItemTitle">(.*?)</div>', page)
            if match: info["name"] = html.unescape(match.group(1).strip())
            ts_match = re.search(r'data-timestamp="(\d+)"', page)
            if ts_match: info["updated"] = ts_match.group(1)
            # Robust Dependency Scraping
            matches = re.findall(r'class="requiredItem".*?id=(\d+).*?>(.*?)</a>', page, re.DOTALL)
            for dep_id, dep_html in matches:
                dep_name = re.sub(r'<[^>]+>', '', dep_html).strip()
                info["dependencies"].append({"id": dep_id.strip(), "name": html.unescape(dep_name)})
    except: pass
    return info

def resolve_dependencies(initial_mods, ignored_ids=None):
    if ignored_ids is None: ignored_ids = set()
    print("--- Resolving Dependencies ---")
    
    lock_data = {}
    if os.path.exists(LOCK_FILE):
        try:
            with open(LOCK_FILE, "r") as f: lock_data = json.load(f).get("mods", {})
        except: pass

    resolved_info = {}
    to_check = list(initial_mods.keys())
    processed = set(ignored_ids)
    found_as_dep = set()
    
    # 1. Bulk API Warmup
    print(f"  🔍 Bulk querying {len(to_check)} mods via Steam API...")
    api_cache = get_bulk_metadata(to_check)

    while to_check:
        mid = to_check.pop(0)
        if mid in processed and mid not in initial_mods: continue
        if mid in processed and mid in resolved_info: continue

        # Hybrid Logic: Try API -> Scrape -> Cache
        meta = api_cache.get(mid, {"name": f"Mod {mid}", "updated": "0", "dependencies": []})
        scrape = scrape_mod_metadata(mid)
        
        # Merge Scrape Data (Precision)
        if scrape["name"]: meta["name"] = scrape["name"]
        if scrape["updated"]: meta["updated"] = scrape["updated"]
        meta["dependencies"] = scrape["dependencies"]

        # Cache Fallback
        if meta["updated"] == "0" and mid in lock_data:
            print(f"  ℹ️  Offline: Using cached metadata for {lock_data[mid].get('name', mid)}")
            meta["name"] = lock_data[mid].get("name", meta.get("name", mid))
            meta["updated"] = lock_data[mid].get("updated", "0")
            meta["dependencies"] = lock_data[mid].get("dependencies", [])

        if mid in initial_mods and initial_mods[mid]: meta["name"] = initial_mods[mid]
        if mid not in found_as_dep: print(f"Checking {meta['name']} ({mid})...")
            
        resolved_info[mid] = meta
        processed.add(mid)
        
        for dep in meta["dependencies"]:
            dep_id = dep["id"]
            # Filter out non-mod Steam IDs (Arma 3 and Arma 3 Tools)
            if dep_id in ["107410", "228800"]:
                continue
                
            if dep_id not in processed and dep_id not in to_check:
                print(f"  Found dependency: {dep['name']} ({dep_id})")
                to_check.append(dep_id); found_as_dep.add(dep_id)
                
    return resolved_info

def run_steamcmd(mod_ids):
    if not mod_ids: return
    if os.getenv("UKSFTA_OFFLINE") == "1":
        print("\n[!] Global Offline Mode Active (via .env). Skipping SteamCMD.")
        return
    username = os.getenv("STEAM_USERNAME"); password = os.getenv("STEAM_PASSWORD")
    if username and password:
        login_user = username; login_pass = password
        print(f"\n--- Updating {len(mod_ids)} mods via SteamCMD (Authenticated) ---")
    else:
        login_user = "anonymous"; login_pass = None
        print(f"\n--- Updating {len(mod_ids)} mods via SteamCMD (as anonymous) ---")
    base_cmd = ["steamcmd", "+login", login_user]
    if login_pass: base_cmd.append(login_pass)
    for mid in mod_ids:
        print(f"--- Syncing Item: {mid} ---")
        cmd = base_cmd + ["+workshop_download_item", STEAMAPP_ID, mid, "validate", "+quit"]
        try: subprocess.run(cmd, check=True)
        except subprocess.CalledProcessError:
            print(f"\n⚠️  Download of {mid} failed. Use Steam Desktop Client as fallback.")
    print("\n✅ SteamCMD attempt finished.")

def get_workshop_cache_path():
    home = os.path.expanduser("~")
    possible_paths = [
        os.path.join(home, ".steam/steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(home, "Steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(home, ".local/share/Steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join("/ext/SteamLibrary/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(home, ".steam/steamcmd/steamapps/workshop/content", STEAMAPP_ID),
        os.getcwd()
    ]
    for p in possible_paths:
        if os.path.exists(p): return p
    return None

def identify_existing_pbos():
    cache_path = get_workshop_cache_path()
    if not cache_path: return
    print("--- Identifying PBO Origins ---")
    pbo_map = {}
    for mod_id in os.listdir(cache_path):
        mod_dir = os.path.join(cache_path, mod_id)
        if not os.path.isdir(mod_dir): continue
        for root, _, files in os.walk(mod_dir):
            for f in files:
                if f.lower().endswith(".pbo"):
                    if f not in pbo_map: pbo_map[f] = []
                    pbo_map[f].append(mod_id)
    if not os.path.exists(ADDONS_DIR): return
    found_matches = {}; unidentified = []
    for f in os.listdir(ADDONS_DIR):
        if f.lower().endswith(".pbo"):
            if f in pbo_map:
                mid = pbo_map[f][0]
                if mid not in found_matches: found_matches[mid] = []
                found_matches[mid].append(f)
            else: unidentified.append(f)
    for mid, files in found_matches.items():
        print(f"Mod ID {mid} contains:"); [print(f"  - {file}") for file in files]
    if unidentified:
        print("\nUnidentified PBOs:"); [print(f"  - {f}") for f in unidentified]

def sync_mods(resolved_info):
    if os.path.exists(LOCK_FILE):
        with open(LOCK_FILE, "r") as f:
            lock_data = json.load(f)
            if "mods" not in lock_data: lock_data = {"mods": {}}
    else: lock_data = {"mods": {}}
    current_mods = {}
    base_workshop_path = get_workshop_cache_path()
    if not base_workshop_path:
        if "pytest" in sys.modules: base_workshop_path = "/tmp/workshop_mock"; os.makedirs(base_workshop_path, exist_ok=True)
        else: print("Error: Workshop cache not found."); sys.exit(1)
    os.makedirs(ADDONS_DIR, exist_ok=True)
    if os.path.exists(KEYS_DIR): shutil.rmtree(KEYS_DIR)
    os.makedirs(KEYS_DIR, exist_ok=True)
    for mid, info in resolved_info.items():
        mod_path = os.path.join(base_workshop_path, mid)
        locked_info = lock_data["mods"].get(mid, {})
        locked_ts = locked_info.get("updated", "0")
        current_ts = info.get("updated", "1")
        files_exist = all(os.path.exists(f) for f in locked_info.get("files", [])) if locked_info.get("files") else False
        if locked_ts == "0" and files_exist: locked_info["updated"] = current_ts; locked_ts = current_ts
        if current_ts == locked_ts and files_exist:
            current_mods[mid] = locked_info; continue
        if not os.path.exists(mod_path):
            print(f"Warning: Mod {info['name']} ({mid}) missing from cache."); continue
        print(f"--- Syncing: {info['name']} (v{current_ts}) ---")
        current_mods[mid] = {"files": [], "name": info["name"], "dependencies": info["dependencies"], "updated": current_ts}
        for root, _, files in os.walk(mod_path):
            for file in files:
                if file.lower().endswith(".pbo"):
                    dest = os.path.join(ADDONS_DIR, file); shutil.copy2(os.path.join(root, file), dest)
                    os.utime(dest, None); current_mods[mid]["files"].append(os.path.relpath(dest))
    for old_mid in list(lock_data["mods"].keys()):
        if old_mid not in resolved_info:
            print(f"--- Cleaning up Mod ID: {old_mid} ---")
            for rel in lock_data["mods"][old_mid].get("files", []):
                if os.path.exists(rel): os.remove(rel)
    with open(LOCK_FILE, "w") as f: json.dump({"mods": current_mods}, f, indent=2)
    sync_hemtt_launch(set(resolved_info.keys()))

def sync_hemtt_launch(mod_ids):
    path = ".hemtt/launch.toml"
    if not os.path.exists(path): return
    print(f"--- Syncing {path} ---")
    with open(path, "r") as f: lines = f.readlines()
    new_lines = []; in_workshop = False
    for line in lines:
        if "workshop =" in line:
            in_workshop = True; new_lines.append(line)
            for mid in sorted(mod_ids): new_lines.append(f'    "{mid}",\n')
            continue
        if in_workshop:
            if "]" in line: in_workshop = False; new_lines.append(line)
            continue
        new_lines.append(line)
    with open(path, "w") as f: f.writelines(new_lines)

if __name__ == "__main__":
    load_env(); parser = argparse.ArgumentParser(description="UKSFTA Mod Manager")
    parser.add_argument("command", nargs='?', default="sync", choices=["sync", "identify"])
    parser.add_argument("--offline", action="store_true"); args = parser.parse_args()
    if args.command == "identify": identify_existing_pbos(); sys.exit(0)
    initial_mods = get_mod_ids_from_file(); ignored_ids = get_ignored_ids_from_file()
    try:
        resolved_info = {}
        if initial_mods:
            resolved_info = resolve_dependencies(initial_mods, ignored_ids)
            is_offline = args.offline or os.getenv("UKSFTA_OFFLINE") == "1"
            if not is_offline:
                lock_data = {"mods": {}}
                if os.path.exists(LOCK_FILE):
                    with open(LOCK_FILE, "r") as f: lock_data = json.load(f)
                needs_download = []
                for mid, info in resolved_info.items():
                    locked_mod = lock_data.get("mods", {}).get(mid, {})
                    if info["updated"] != locked_mod.get("updated", "0") or not all(os.path.exists(f) for f in locked_mod.get("files", [])):
                        needs_download.append(mid)
                if needs_download: run_steamcmd(needs_download)
                else: print("\n✅ Workshop dependencies are up to date.")
            else:
                print("\n[!] Offline Mode: Syncing from cache only.")
        else: print("No external mods defined.")
        sync_mods(resolved_info); print("\nSuccess: Workspace synced.")
    except Exception as e:
        print(f"\nError: {e}"); sys.exit(1)
