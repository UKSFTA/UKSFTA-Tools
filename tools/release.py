import os
import sys
import re
import subprocess
import shutil
import getpass
import json
import glob
import urllib.request
import html
import argparse
import multiprocessing
try:
    from rich.console import Console
    from rich import print as rprint
    HAS_RICH = True
except ImportError:
    HAS_RICH = False
    rprint = print

# Configuration
PROJECT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
def find_version_file():
    # Recursively find the first script_version.hpp in addons/
    addons_dir = os.path.join(PROJECT_ROOT, "addons")
    for root, _, files in os.walk(addons_dir):
        if "script_version.hpp" in files:
            return os.path.join(root, "script_version.hpp")
    return None

VERSION_FILE = find_version_file()
HEMTT_OUT = os.path.join(PROJECT_ROOT, ".hemttout")
# Workshop expects the RAW contents (addons, keys etc) at the root
STAGING_DIR = os.path.join(HEMTT_OUT, "release")
PROJECT_TOML = os.path.join(PROJECT_ROOT, ".hemtt", "project.toml")
LOCK_FILE = "mods.lock"

def load_env():
    env_path = os.path.join(PROJECT_ROOT, ".env")
    if os.path.exists(env_path):
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

def get_current_version():
    if not os.path.exists(VERSION_FILE):
        return "0.0.0", (0, 0, 0)
    with open(VERSION_FILE, "r") as f:
        content = f.read()
    
    major = re.search(r"#define\s+MAJOR\s+(\d+)", content).group(1)
    minor = re.search(r"#define\s+MINOR\s+(\d+)", content).group(1)
    patch = re.search(r"#define\s+PATCHLVL\s+(\d+)", content).group(1)
    return f"{major}.{minor}.{patch}", (int(major), int(minor), int(patch))

def bump_version(part="patch"):
    version_str, (major, minor, patch) = get_current_version()
    
    if part == "major":
        major += 1
        minor = 0
        patch = 0
    elif part == "minor":
        minor += 1
        patch = 0
    else: # patch
        patch += 1
        
    new_version = f"{major}.{minor}.{patch}"
    print(f"Bumping version: {version_str} -> {new_version}")
    
    with open(VERSION_FILE, "r") as f:
        content = f.read()
        
    content = re.sub(r"#define\s+MAJOR\s+\d+", f"#define MAJOR {major}", content)
    content = re.sub(r"#define\s+MINOR\s+\d+", f"#define MINOR {minor}", content)
    content = re.sub(r"#define\s+PATCHLVL\s+\d+", f"#define PATCHLVL {patch}", content)
    
    with open(VERSION_FILE, "w") as f:
        f.write(content)
        
    return new_version

def get_workshop_config():
    config = {
        "id": "0",
        "tags": ["Mod", "Addon"]
    }
    if os.path.exists(PROJECT_TOML):
        with open(PROJECT_TOML, "r") as f:
            for line in f:
                if "workshop_id" in line:
                    val = line.split("=")[1].strip().strip('"')
                    if val: config["id"] = val
                if "workshop_tags" in line:
                    tags_match = re.search(r"\[(.*?)\]", line)
                    if tags_match:
                        config["tags"] = [t.strip().strip('"').strip("'") for t in tags_match.group(1).split(",")]
    return config

def generate_content_list():
    lock_data = {"mods": {}}
    if os.path.exists(LOCK_FILE):
        with open(LOCK_FILE, "r") as f:
            lock_data = json.load(f)
            if "mods" not in lock_data:
                lock_data = {"mods": {}}

    if not os.path.exists("mod_sources.txt"):
        return "[*] [i]No external content listed.[/i]"
    
    content_list = ""
    with open("mod_sources.txt", "r") as f:
        for line in f:
            clean_line = line.strip()
            if not clean_line or clean_line.startswith("#"):
                continue
            
            # Respect [ignore] block
            if "[ignore]" in clean_line.lower() or "[ignored]" in clean_line.lower():
                break

            # Respect inline ignore
            if "ignore=" in clean_line.lower() or "@ignore" in clean_line.lower():
                continue

            match = re.search(r"(?:id=)?(\d{8,})", clean_line)
            if not match: continue
            mid = match.group(1)
            
            tag = ""
            if "#" in clean_line:
                tag = clean_line.split("#", 1)[1].strip()
            
            mod_info = lock_data["mods"].get(mid, {})
            mod_name = tag if tag else mod_info.get("name", f"Mod {mid}")
            mod_url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={mid}"

            display_name = mod_name
            category = ""
            if "|" in mod_name:
                parts = mod_name.split("|")
                category = parts[0].strip()
                display_name = parts[1].strip()

            content_list += f"[*] [url={mod_url}][b]{display_name}[/b][/url]"
            if category:
                content_list += f" ({category})"
            
            deps = mod_info.get("dependencies", [])
            if deps:
                content_list += "\n[list]\n"
                for dep in deps:
                    dep_url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={dep['id']}"
                    content_list += f"[*] [i]Dependency Included:[/i] [url={dep_url}]{dep['name']}[/url]\n"
                content_list += "[/list]\n"
            else:
                content_list += "\n"
    
    return content_list if content_list else "[*] [i]Content list pending update.[/i]"

def generate_changelog(last_tag):
    try:
        if last_tag == "HEAD":
            cmd = ["git", "log", "--oneline", "--no-merges"]
        else:
            cmd = ["git", "log", f"{last_tag}..HEAD", "--oneline", "--no-merges"]
        result = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return result.stdout.strip()
    except:
        return "Maintenance update."

def create_vdf(app_id, workshop_id, content_path, changelog, preview_image=None):
    description = ""
    if os.path.exists("workshop_description.txt"):
        with open("workshop_description.txt", "r") as f:
            description = f.read()

    included_content = generate_content_list()
    description = description.replace("{{INCLUDED_CONTENT}}", included_content)

    config = get_workshop_config()
    tags_vdf = ""
    for i, tag in enumerate(config["tags"]):
        tags_vdf += f'        "{i}" "{tag}"\n'

    preview_line = f'"previewfile" "{preview_image}"' if preview_image else ""
    
    vdf_content = f"""
"workshopitem"
{{
    "appid" "{app_id}"
    "publishedfileid" "{workshop_id}"
    "contentfolder" "{content_path}"
    "changenote" "{changelog}"
    "description" "{description}"
    "tags"
    {{
{tags_vdf}    }}
    {preview_line}
}}
"""
    vdf_path = os.path.join(HEMTT_OUT, "upload.vdf")
    with open(vdf_path, "w") as f:
        f.write(vdf_content)
    return vdf_path

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Release Tool")
    group = parser.add_mutually_exclusive_group()
    group.add_argument("-p", "--patch", action="store_true", help="Bump patch version")
    group.add_argument("-m", "--minor", action="store_true", help="Bump minor version")
    group.add_argument("-M", "--major", action="store_true", help="Bump major version")
    group.add_argument("-n", "--none", action="store_true", help="Don't bump version")
    
    parser.add_argument("-t", "--tag", action="store_true", help="Force git tagging")
    parser.add_argument("-y", "--yes", action="store_true", help="Skip confirmation for tagging (implies --tag)")
    parser.add_argument("-j", "--threads", type=int, default=multiprocessing.cpu_count(), help="Number of threads for HEMTT (default: all cores)")
    parser.add_argument("--dry-run", action="store_true", help="Generate VDF and validate but do not upload")
    
    args = parser.parse_args()

    load_env()
    if not shutil.which("hemtt"):
        print("Error: 'hemtt' not found.")
        sys.exit(1)
    if not args.dry_run and not shutil.which("steamcmd"):
        print("Error: 'steamcmd' not found.")
        sys.exit(1)

    current_v_str, _ = get_current_version()
    print(f"Current version: {current_v_str}")
    
    confirm = None
    if args.patch: confirm = 'p'
    elif args.minor: confirm = 'm'
    elif args.major: confirm = 'major'
    elif args.none: confirm = 'n'
    
    if confirm is None:
        confirm = input("Bump version? [p]atch/[m]inor/[M]ajor/[n]one: ").lower()
    
    new_version = current_v_str
    if confirm in ['p', 'm', 'major']:
        part = "patch"
        if confirm == 'm': part = "minor"
        if confirm == 'major': part = "major"
        new_version = bump_version(part)
        if not args.dry_run:
            subprocess.run(["git", "add", VERSION_FILE], check=True)
            subprocess.run(["git", "commit", "-S", "-m", f"chore: bump version to {new_version}"], check=True)
        else:
            print(f"[DRY-RUN] Would commit version bump to {new_version}")

    # USE THE ROBUST WRAPPER FOR BUILDING
    print(f"Running Robust Release Build...")
    subprocess.run(["bash", "build.sh", "release", "-t", str(args.threads)], check=True)

    # Locate the newly created ZIP for GitHub
    possible_zips = glob.glob(os.path.join(PROJECT_ROOT, "releases", "*.zip"))
    
    # Also check the central unit hub
    central_hub = os.path.join(PROJECT_ROOT, "..", "UKSFTA-Tools", "all_releases")
    if os.path.exists(central_hub):
        possible_zips += glob.glob(os.path.join(central_hub, "*.zip"))

    if not possible_zips:
        print("Error: No release zip found in local releases/ or central hub.")
        sys.exit(1)
    
    # Get the absolute newest file by creation time
    latest_zip = max(possible_zips, key=os.path.getctime)
    print(f"Using release package: {os.path.basename(latest_zip)}")
    
    ws_config = get_workshop_config()
    workshop_id = ws_config["id"]
    if not workshop_id or workshop_id == "0":
        if not args.dry_run:
            workshop_id = input("Enter Workshop ID to update: ").strip()
        else:
            workshop_id = "123456789 (Simulated)"
        
    try:
        last_tag = subprocess.check_output(["git", "describe", "--tags", "--abbrev=0"]).decode().strip()
    except:
        try:
            last_tag = subprocess.check_output(["git", "rev-list", "--max-parents=0", "HEAD"]).decode().strip()
        except:
            last_tag = "HEAD"
        
    changelog = generate_changelog(last_tag)
    # Upload from .hemttout/release which contains the normalized raw files
    vdf_path = create_vdf("107410", workshop_id, STAGING_DIR, changelog)
    
    if args.dry_run:
        print("\n" + "="*60)
        if HAS_RICH:
            rprint("       [bold cyan]STEAM WORKSHOP MOCK PREVIEW[/bold cyan]")
        else:
            print("       STEAM WORKSHOP MOCK PREVIEW")
        print("="*60)
        ws_config = get_workshop_config()
        if HAS_RICH:
            rprint(f"[bold]Workshop ID:[/bold]  {workshop_id}")
            rprint(f"[bold]Version:[/bold]      {new_version}")
            rprint(f"[bold]Tags:[/bold]         {', '.join(ws_config['tags'])}")
            rprint("\n[bold cyan]--- Description Preview ---[/bold cyan]")
        else:
            print(f"Workshop ID:  {workshop_id}")
            print(f"Version:      {new_version}")
            print(f"Tags:         {', '.join(ws_config['tags'])}")
            print("\n--- Description Preview ---")
            
        # Mock the description replacement
        desc = ""
        if os.path.exists("workshop_description.txt"):
            with open("workshop_description.txt", "r") as f:
                desc = f.read()
        desc = desc.replace("{{INCLUDED_CONTENT}}", generate_content_list())
        # Strip BBCode for the preview so it's readable in terminal
        preview_desc = re.sub(r"\[.*?\]", "", desc)
        print(preview_desc.strip())
        
        if HAS_RICH:
            rprint("\n[bold cyan]--- Changelog ---[/bold cyan]")
        else:
            print("\n--- Changelog ---")
        print(changelog if changelog else "Initial release.")
        print("="*60)

        print("\n[DRY-RUN] Build complete. Integrity check follows...")
        # Use our new checker tool
        subprocess.run([sys.executable, "tools/mod_integrity_checker.py", STAGING_DIR, "--unsigned"])
        print("\n[DRY-RUN] Upload skipped. Ready for production.")
        return

    print("\n--- Steam Workshop Upload ---")
    username = os.getenv("STEAM_USERNAME")
    password = os.getenv("STEAM_PASSWORD")

    if not username:
        username = input("Steam Username: ").strip()
    
    cmd = ["steamcmd", "+login", username]
    if password:
        cmd.append(password)
    cmd.extend(["+workshop_build_item", vdf_path, "+quit"])
    
    print(f"Launching SteamCMD for user: {username}...")
    
    try:
        subprocess.run(cmd, check=True)
        print("\nSUCCESS: Mod updated on Workshop.")
        
        do_tag = args.tag or args.yes
        if not do_tag:
            if confirm != 'n':
                do_tag = True
            else:
                do_tag = input("Tag this release in Git? [y/N]: ").lower() == 'y'

        if do_tag:
            tag_name = f"v{new_version}"
            branch = subprocess.check_output(["git", "rev-parse", "--abbrev-ref", "HEAD"]).decode().strip()
            subprocess.run(["git", "tag", "-a", tag_name, "-m", f"Release {new_version}", "-f"], check=True)
            subprocess.run(["git", "push", "origin", branch, "--tags", "-f"], check=False)

            if shutil.which("gh"):
                print(f"Creating GitHub Release for {tag_name}...")
                gh_cmd = ["gh", "release", "create", tag_name, latest_zip, "--title", f"Release {new_version}", "--notes", changelog, "--latest"]
                subprocess.run(gh_cmd, check=False)
            
    except subprocess.CalledProcessError as e:
        print(f"\nError during upload: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
