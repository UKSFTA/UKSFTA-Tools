import os
import re
import subprocess
import shutil
import json
import sys
import urllib.request
import html

# Configuration
MOD_SOURCES_FILE = "mod_sources.txt"
LOCK_FILE = "mods.lock"
ADDONS_DIR = "addons"
KEYS_DIR = "keys"
STEAMAPP_ID = "107410"  # Arma 3

def get_mod_ids_from_file():
    mods = {}
    if not os.path.exists(MOD_SOURCES_FILE):
        return mods
    
    with open(MOD_SOURCES_FILE, "r") as f:
        for line in f:
            clean_line = line.strip()
            if not clean_line or clean_line.startswith("#"):
                continue
            
            # Skip lines that are clearly for ignoring
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
    
    with open(MOD_SOURCES_FILE, "r") as f:
        for line in f:
            clean_line = line.strip().lower()
            if not clean_line or clean_line.startswith("#"):
                continue
            
            if "ignore=" in clean_line or "@ignore" in clean_line:
                matches = re.findall(r"(\d{8,})", clean_line)
                for mid in matches:
                    ignored.add(mid)
    return ignored

def get_workshop_metadata(mod_id):
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={mod_id}"
    info = {"name": f"Mod {mod_id}", "dependencies": []}
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as response:
            page = response.read().decode('utf-8')
            
            # Title
            match = re.search(r'<div class="workshopItemTitle">(.*?)</div>', page)
            if match:
                info["name"] = html.unescape(match.group(1).strip())
            
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
    
    while to_check:
        mid = to_check.pop(0)
        if mid in processed and mid not in initial_mods:
            continue
            
        if mid in processed and mid in resolved_info:
            continue

        print(f"Checking {mid}...")
        meta = get_workshop_metadata(mid)
        if mid in initial_mods and initial_mods[mid]:
            meta["name"] = initial_mods[mid]
            
        resolved_info[mid] = meta
        processed.add(mid)
        
        for dep in meta["dependencies"]:
            if dep["id"] not in processed:
                print(f"  Found dependency: {dep['name']} ({dep['id']})")
                to_check.append(dep["id"])
                
    return resolved_info

def run_steamcmd(mod_ids):
    if not mod_ids:
        return
    cmd = ["steamcmd", "+login", "anonymous"]
    for mid in mod_ids:
        cmd.extend(["+workshop_download_item", STEAMAPP_ID, mid])
    cmd.append("+quit")
    print(f"\n--- Updating {len(mod_ids)} mods via SteamCMD ---")
    subprocess.run(cmd, check=True)

def sync_mods(resolved_info):
    if os.path.exists(LOCK_FILE):
        with open(LOCK_FILE, "r") as f:
            lock_data = json.load(f)
            if "mods" not in lock_data:
                lock_data = {"mods": {}}
    else:
        lock_data = {"mods": {}}

    current_mods = {}
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
        print("Error: Could not find Steam Workshop download directory.")
        sys.exit(1)

    os.makedirs(ADDONS_DIR, exist_ok=True)
    os.makedirs(KEYS_DIR, exist_ok=True)

    for mid, info in resolved_info.items():
        mod_path = os.path.join(base_workshop_path, mid)
        if not os.path.exists(mod_path):
            print(f"Warning: Mod {info['name']} ({mid}) not found in workshop cache.")
            continue
            
        print(f"--- Syncing: {info['name']} ---")
        current_mods[mid] = {
            "files": [], 
            "name": info["name"],
            "dependencies": info["dependencies"]
        }
        
        for root, dirs, files in os.walk(mod_path):
            for file in files:
                file_lower = file.lower()
                src_path = os.path.join(root, file)
                if file_lower.endswith((".pbo", ".bisign")):
                    dest_path = os.path.join(ADDONS_DIR, file)
                    shutil.copy2(src_path, dest_path)
                    current_mods[mid]["files"].append(os.path.relpath(dest_path))
                elif file_lower.endswith(".bikey"):
                    dest_path = os.path.join(KEYS_DIR, file)
                    shutil.copy2(src_path, dest_path)
                    current_mods[mid]["files"].append(os.path.relpath(dest_path))

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
    initial_mods = get_mod_ids_from_file()
    ignored_ids = get_ignored_ids_from_file()
    
    if not initial_mods:
        print("No mod IDs found in mod_sources.txt")
        sys.exit(0)
    
    try:
        resolved_info = resolve_dependencies(initial_mods, ignored_ids)
        run_steamcmd(set(resolved_info.keys()))
        sync_mods(resolved_info)
        print("\nSuccess: All mods and dependencies synced.")
    except Exception as e:
        print(f"\nError: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)
