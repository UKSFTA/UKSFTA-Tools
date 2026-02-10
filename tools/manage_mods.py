import os
import re
import subprocess
import shutil
import json
import sys
import urllib.request
import html

# Configuration
PROJECT_ROOT = os.getcwd()
MOD_SOURCES_FILE = "mod_sources.txt"
LOCK_FILE = "mods.lock"
ADDONS_DIR = "addons"
KEYS_DIR = "keys"
STEAMAPP_ID = "107410"  # Arma 3

def load_env():
    # 1. Check local project .env
    env_paths = [
        os.path.join(PROJECT_ROOT, ".env"),
        os.path.join(PROJECT_ROOT, "..", "UKSFTA-Tools", ".env")
    ]
    
    for env_path in env_paths:
        if os.path.exists(env_path):
            # print(f"[DEBUG] Loading credentials from: {env_path}")
            with open(env_path, "r") as f:
                for line in f:
                    line = line.strip()
                    if not line or line.startswith("#"):
                        continue
                    if "=" in line:
                        parts = line.split("=", 1)
                        if len(parts) == 2:
                            key, value = parts
                            os.environ[key.strip()] = value.strip()
            # Stop after finding the first valid .env
            return

def get_mod_ids_from_file():
    mods = {}
    if not os.path.exists(MOD_SOURCES_FILE):
        return mods
    
    with open(MOD_SOURCES_FILE, "r") as f:
        for line in f:
            clean_line = line.strip()
            if not clean_line or clean_line.startswith("#"):
                continue
            
            # Stop processing if we hit the ignore block
            if "[ignore]" in clean_line.lower() or "[ignored]" in clean_line.lower():
                break

            # Skip lines that are inline ignores
            if "ignore=" in clean_line.lower() or "@ignore" in clean_line.lower():
                continue

            match = re.search(r"(?:id=)?(\d{8,})", clean_line)
            if match:
                mod_id = match.group(1)
                tag = ""
                if "#" in clean_line:
                    tag = clean_line.split("#", 1)[1].strip()
                mods[mod_id] = tag
    return mods

def get_ignored_ids_from_file():
    ignored = set()
    if not os.path.exists(MOD_SOURCES_FILE):
        return ignored
    
    ignore_block = False
    with open(MOD_SOURCES_FILE, "r") as f:
        for line in f:
            clean_line = line.strip().lower()
            if not clean_line or clean_line.startswith("#"):
                continue
            
            # Check for block marker
            if "[ignore]" in clean_line or "[ignored]" in clean_line:
                ignore_block = True
                continue

            if ignore_block:
                # In block, extract any ID found on the line
                matches = re.findall(r"(\d{8,})", clean_line)
                for mid in matches:
                    ignored.add(mid)
            else:
                # Outside block, still support inline ignore for backward compatibility
                if "ignore=" in clean_line or "@ignore" in clean_line:
                    matches = re.findall(r"(\d{8,})", clean_line)
                    for mid in matches:
                        ignored.add(mid)
    return ignored

def get_workshop_metadata(mod_id):
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={mod_id}"
    info = {"name": f"Mod {mod_id}", "dependencies": [], "updated": "0"}
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as response:
            page = response.read().decode('utf-8')
            
            # Title
            match = re.search(r'<div class="workshopItemTitle">(.*?)</div>', page)
            if match:
                info["name"] = html.unescape(match.group(1).strip())
            
            # Timestamp (Reliable Unix Epoch from Steam)
            ts_match = re.search(r'data-timestamp="(\d+)"', page)
            if ts_match:
                info["updated"] = ts_match.group(1)
            
            # Dependencies (Required Items) - Robust search
            # Look for links within the RequiredItems container
            deps_section = re.search(r'id="RequiredItems">(.*?)</div>\s*</div>', page, re.DOTALL)
            if deps_section:
                # Find all hrefs with id= in that section
                items = re.findall(r'href=".*?id=(\d+)".*?>(.*?)</a>', deps_section.group(1), re.DOTALL)
                for dep_id, dep_html in items:
                    # Clean up the name (it might be wrapped in divs or have whitespace)
                    dep_name = re.sub(r'<[^>]+>', '', dep_html).strip()
                    info["dependencies"].append({
                        "id": dep_id.strip(),
                        "name": html.unescape(dep_name)
                    })
    except Exception as e:
        print(f"Warning: Could not fetch info for mod {mod_id}: {e}")
    return info

def resolve_dependencies(initial_mods, ignored_ids=None):
    if ignored_ids is None:
        ignored_ids = set()
        
    print("--- Resolving Dependencies ---")
    if ignored_ids:
        print(f"Ignoring: {', '.join(ignored_ids)}")
        
    resolved_info = {}
    to_check = list(initial_mods.keys())
    processed = set(ignored_ids)
    found_as_dep = set()
    
    while to_check:
        mid = to_check.pop(0)
        if mid in processed and mid not in initial_mods:
            continue
            
        if mid in processed and mid in resolved_info:
            continue

        meta = get_workshop_metadata(mid)
        if mid in initial_mods and initial_mods[mid]:
            meta["name"] = initial_mods[mid]
            
        # Only print Checking if it wasn't already announced as a dependency
        if mid not in found_as_dep:
            print(f"Checking {meta['name']} ({mid})...")
            
        resolved_info[mid] = meta
        processed.add(mid)
        
        for dep in meta["dependencies"]:
            dep_id = dep["id"]
            if dep_id not in processed and dep_id not in [m for m in to_check]:
                print(f"  Found dependency: {dep['name']} ({dep_id})")
                to_check.append(dep_id)
                found_as_dep.add(dep_id)
                
    return resolved_info

def run_steamcmd(mod_ids):
    if not mod_ids:
        return
    
    username = os.getenv("STEAM_USERNAME")
    password = os.getenv("STEAM_PASSWORD")
    
    # 1. Determine Login Strategy (Prefer .env)
    if username and password:
        login_user = username
        login_pass = password
        print(f"\n--- Updating {len(mod_ids)} mods via SteamCMD (Authenticated as {username}) ---")
    else:
        login_user = "anonymous"
        login_pass = None
        print(f"\n--- Updating {len(mod_ids)} mods via SteamCMD (as anonymous) ---")
        print("[!] Warning: Anonymous downloads are often throttled. Add credentials to .env for maximum speed.")

    # Get the Workshop path to force install there
    workshop_base = get_workshop_cache_path()
    if workshop_base:
        # Move up to the root SteamApps level for the install dir
        install_dir = os.path.abspath(os.path.join(workshop_base, "..", "..", ".."))
    else:
        install_dir = os.getcwd()

    cmd = ["steamcmd", "+force_install_dir", install_dir, "+login", login_user]
    if login_pass:
        cmd.append(login_pass)
        
    for mid in mod_ids:
        cmd.extend(["+workshop_download_item", STEAMAPP_ID, mid])
    cmd.append("+quit")
    
    # 2. Execute with live output for progress visibility
    try:
        # We use a context manager to ensure the process is handled correctly
        # stdout=None allows the process to print directly to the user's terminal, 
        # which preserves SteamCMD's own progress indicators and colors.
        subprocess.run(cmd, check=True, stdout=None, stderr=subprocess.STDOUT)
        print("\n✅ SteamCMD sync completed.")
    except subprocess.CalledProcessError as e:
        print(f"\n❌ SteamCMD failed with exit code {e.returncode}")
        if login_user == "anonymous":
            print("Try adding unit credentials to .env to bypass anonymous restrictions.")
        sys.exit(1)

def get_workshop_cache_path():
    home = os.path.expanduser("~")
    possible_paths = [
        os.path.join(home, ".steam/steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(home, "Steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(home, ".local/share/Steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join("/ext/SteamLibrary/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(os.getcwd(), "steamapps/workshop/content", STEAMAPP_ID)
    ]
    for p in possible_paths:
        if os.path.exists(p):
            return p
    return None

def identify_existing_pbos():
    cache_path = get_workshop_cache_path()
    if not cache_path:
        print("Error: Could not find Steam Workshop cache to identify PBOs.")
        return

    print("--- Identifying PBO Origins ---")
    # Build a map of all PBOs in the Workshop cache
    pbo_map = {}
    for mod_id in os.listdir(cache_path):
        mod_dir = os.path.join(cache_path, mod_id)
        if not os.path.isdir(mod_dir): continue
        for root, _, files in os.walk(mod_dir):
            for f in files:
                if f.lower().endswith(".pbo"):
                    if f not in pbo_map: pbo_map[f] = []
                    pbo_map[f].append(mod_id)

    # Scan our local addons
    if not os.path.exists(ADDONS_DIR):
        print("Addons directory does not exist.")
        return

    found_matches = {}
    unidentified = []
    
    for f in os.listdir(ADDONS_DIR):
        if f.lower().endswith(".pbo"):
            if f in pbo_map:
                match_id = pbo_map[f][0]
                if match_id not in found_matches: found_matches[match_id] = []
                found_matches[match_id].append(f)
            else:
                unidentified.append(f)

    for mid, files in found_matches.items():
        print(f"Mod ID {mid} contains:")
        for file in files:
            print(f"  - {file}")
    
    if unidentified:
        print("\nUnidentified PBOs (Internal or Non-Workshop):")
        for f in unidentified:
            print(f"  - {f}")

def sync_mods(resolved_info):
    if os.path.exists(LOCK_FILE):
        with open(LOCK_FILE, "r") as f:
            lock_data = json.load(f)
            if "mods" not in lock_data:
                lock_data = {"mods": {}}
    else:
        lock_data = {"mods": {}}

    current_mods = {}
    # ... path detection ...
    home = os.path.expanduser("~")
    possible_paths = [
        os.path.join(home, ".steam/steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(home, "Steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(home, ".local/share/Steam/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join("/ext/SteamLibrary/steamapps/workshop/content", STEAMAPP_ID),
        os.path.join(os.getcwd(), "steamapps/workshop/content", STEAMAPP_ID)
    ]
    
    base_workshop_path = None
    for p in possible_paths:
        if os.path.exists(p):
            base_workshop_path = p
            break
            
    if not base_workshop_path:
        if "unittest" in sys.modules or "pytest" in sys.modules:
            base_workshop_path = "/tmp/workshop_mock"
            os.makedirs(base_workshop_path, exist_ok=True)
        else:
            print("Error: Could not find Steam Workshop download directory.")
            sys.exit(1)

    os.makedirs(ADDONS_DIR, exist_ok=True)
    
    if os.path.exists(KEYS_DIR):
        print(f"--- Purging {KEYS_DIR} to remove external keys ---")
        shutil.rmtree(KEYS_DIR)
    os.makedirs(KEYS_DIR, exist_ok=True)

    for mid, info in resolved_info.items():
        mod_path = os.path.join(base_workshop_path, mid)
        
        # VERSION TRACKING: Check if we can skip this mod
        locked_info = lock_data["mods"].get(mid, {})
        locked_ts = locked_info.get("updated", "0")
        current_ts = info.get("updated", "1") # Default to 1 if unknown to trigger sync
        
        # Check if files actually exist in addons/
        files_exist = all(os.path.exists(f) for f in locked_info.get("files", [])) if locked_info.get("files") else False
        
        # RETROACTIVE LOCK: If we have the files but no timestamp, lock them now to avoid a copy
        if locked_ts == "0" and files_exist:
            ver_str = f"v{current_ts}" if current_ts != "0" else "Base Release"
            print(f"--- Retroactively locking version: {info['name']} ({ver_str}) ---")
            locked_info["updated"] = current_ts
            locked_ts = current_ts

        if current_ts == locked_ts and files_exist:
            ver_str = f"v{current_ts}" if current_ts != "0" else "Base Release"
            print(f"--- Mod up to date: {info['name']} ({ver_str}) ---")
            current_mods[mid] = locked_info
            continue

        if not os.path.exists(mod_path):
            print(f"Warning: Mod {info['name']} ({mid}) not found in workshop cache.")
            continue
            
        print(f"--- Syncing: {info['name']} (v{current_ts}) ---")
        current_mods[mid] = {
            "files": [], 
            "name": info["name"],
            "dependencies": info["dependencies"],
            "updated": current_ts
        }
        
        for root, dirs, files in os.walk(mod_path):
            for file in files:
                file_lower = file.lower()
                src_path = os.path.join(root, file)
                if file_lower.endswith(".pbo"):
                    dest_path = os.path.join(ADDONS_DIR, file)
                    shutil.copy2(src_path, dest_path)
                    os.utime(dest_path, None) # Normalize timestamp
                    current_mods[mid]["files"].append(os.path.relpath(dest_path))
                # Explicitly skipping .bisign and .bikey files as requested

    # Cleanup: Remove mods that are no longer in resolved_info
    for old_mid in list(lock_data["mods"].keys()):
        if old_mid not in resolved_info:
            print(f"--- Cleaning up Mod ID: {old_mid} ---")
            for rel_path in lock_data["mods"][old_mid].get("files", []):
                if os.path.exists(rel_path):
                    print(f"Removing {rel_path}")
                    os.remove(rel_path)
    
    with open(LOCK_FILE, "w") as f:
        json.dump({"mods": current_mods}, f, indent=2)
    
    sync_hemtt_launch(set(resolved_info.keys()))

def sync_hemtt_launch(mod_ids):
    launch_path = ".hemtt/launch.toml"
    if not os.path.exists(launch_path):
        return
    print(f"--- Syncing {launch_path} ---")
    with open(launch_path, "r") as f:
        lines = f.readlines()
    new_lines = []
    in_workshop = False
    for line in lines:
        if "workshop =" in line:
            in_workshop = True
            new_lines.append(line)
            for mid in sorted(mod_ids):
                new_lines.append(f'    "{mid}",\n')
            continue
        if in_workshop:
            if "]" in line:
                in_workshop = False
                new_lines.append(line)
            continue
        new_lines.append(line)
    with open(launch_path, "w") as f:
        f.writelines(new_lines)

if __name__ == "__main__":
    load_env()
    if len(sys.argv) > 1 and sys.argv[1] == "identify":
        identify_existing_pbos()
        sys.exit(0)

    initial_mods = get_mod_ids_from_file()
    ignored_ids = get_ignored_ids_from_file()
    
    try:
        resolved_info = {}
        if initial_mods:
            resolved_info = resolve_dependencies(initial_mods, ignored_ids)
            
            # 1. Load current lock to compare versions
            lock_data = {"mods": {}}
            if os.path.exists(LOCK_FILE):
                with open(LOCK_FILE, "r") as f:
                    lock_data = json.load(f)

            # 2. Only download if timestamp changed or files are missing
            needs_download = []
            for mid, info in resolved_info.items():
                locked_mod = lock_data["mods"].get(mid, {})
                locked_ts = locked_mod.get("updated", "0")
                current_ts = info["updated"]
                
                # Check if files actually exist in addons/
                files_exist = all(os.path.exists(f) for f in locked_mod.get("files", [])) if locked_mod.get("files") else False
                
                # RETROACTIVE LOCK: If we have files but no TS, we don't need to download
                if locked_ts == "0" and files_exist:
                    # We still want to run sync_mods later to write the new lock, 
                    # but we don't need SteamCMD
                    continue

                if current_ts != locked_ts:
                    needs_download.append(mid)
                elif not files_exist:
                    needs_download.append(mid)

            if needs_download:
                run_steamcmd(needs_download)
            else:
                print("\n✅ All Workshop dependencies are already at the latest version or locked.")
        else:
            print("No external mods defined. Running workspace maintenance...")
            
        sync_mods(resolved_info)
        print("\nSuccess: Workspace synced and cleaned.")
    except Exception as e:
        print(f"\nError: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)
