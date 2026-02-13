#!/usr/bin/env python3
import requests
import json
import os
import sys
import re
import urllib.request
import math
from datetime import datetime
from pathlib import Path

# Try to import rich for high-fidelity CLI output
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    from rich.text import Text
    from rich.panel import Panel
    USE_RICH = True
except ImportError:
    USE_RICH = False

STEAM_API_URL = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"

def format_size(size_bytes):
    if size_bytes == 0: return "0 B"
    size_name = ("B", "KB", "MB", "GB", "TB")
    i = int(math.floor(math.log(size_bytes, 1024)))
    p = math.pow(1024, i)
    s = round(size_bytes / p, 2)
    return "%s %s" % (s, size_name[i])

def get_mod_info_from_file(file_path):
    mods = {} # ID -> Name
    if not os.path.exists(file_path): return mods
    
    with open(file_path, 'r', errors='ignore') as f:
        content = f.read()
        
        # Standard Arma 3 Preset Pattern
        pattern = r'<tr data-type="ModContainer">\s*<td data-type="DisplayName">([^<]+)</td>.*?id=(\d+)'
        matches = re.findall(pattern, content, re.DOTALL)
        
        for name, mid in matches:
            mods[mid] = name.strip()
            
    return mods

def get_workshop_details(published_ids):
    if not published_ids: return []
    details = []
    id_list = list(published_ids)
    for i in range(0, len(id_list), 100):
        chunk = id_list[i:i + 100]
        data = {"itemcount": len(chunk)}
        for j, pid in enumerate(chunk): data[f"publishedfileids[{j}]"] = pid
        try:
            response = requests.post(STEAM_API_URL, data=data, timeout=15)
            response.raise_for_status()
            details.extend(response.json().get("response", {}).get("publishedfiledetails", []))
        except Exception as e:
            print(f"  ⚠️  API Error: {e}")
    return details

def scrape_details_fallback(published_id):
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    details = {"size": 0, "title": f"Mod {published_id}"}
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=5) as response:
            html_content = response.read().decode('utf-8')
            title_match = re.search(r'<div class="workshopItemTitle">(.*?)</div>', html_content)
            if title_match: details["title"] = title_match.group(1).strip()
            all_stats = re.findall(r'<div class="detailsStatRight">(.*?)</div>', html_content)
            if all_stats:
                size_str = all_stats[0].strip()
                num_match = re.search(r'[\d\.,]+', size_str)
                if num_match:
                    num = float(num_match.group().replace(',', ''))
                    if "GB" in size_str: details["size"] = int(num * 1024**3)
                    elif "MB" in size_str: details["size"] = int(num * 1024**2)
                    elif "KB" in size_str: details["size"] = int(num * 1024)
    except: pass
    return details

def main():
    console = Console() if USE_RICH else None
    input_file = sys.argv[1] if len(sys.argv) > 1 else "mod_sources.txt"
    
    if not os.path.exists(input_file):
        print(f"Usage: modlist_size.py <file>")
        return

    if USE_RICH:
        console.print(Panel.fit(f"📊 [bold blue]Modlist Size Calculator[/bold blue]\n[dim]{input_file}[/dim]", border_style="blue"))

    mod_map = get_mod_info_from_file(input_file)
    if not mod_map:
        # Fallback to raw ID search
        with open(input_file, 'r', errors='ignore') as f:
            raw_ids = re.findall(r'(?:id=)?(\d{8,})', f.read())
            mod_map = {mid: f"Mod {mid}" for mid in raw_ids}

    if not mod_map:
        print("❌ No mods detected.")
        return

    print(f"🔍 Found {len(mod_map)} mods. Querying Steam...")
    details_list = get_workshop_details(set(mod_map.keys()))
    
    results = []
    total_bytes = 0
    processed_ids = set()
    
    for detail in details_list:
        pid = detail.get("publishedfileid")
        if not pid: continue
        processed_ids.add(pid)
        
        name = mod_map.get(pid, detail.get("title", f"Mod {pid}"))
        size = int(detail.get("file_size", 0))
        if size == 0:
            fallback = scrape_details_fallback(pid)
            size = fallback["size"]
            
        total_bytes += size
        results.append({"name": name, "size": size, "id": pid})

    # Catch IDs that API completely missed
    for pid, name in mod_map.items():
        if pid not in processed_ids:
            fallback = scrape_details_fallback(pid)
            total_bytes += fallback["size"]
            results.append({"name": name, "size": fallback["size"], "id": pid})

    if USE_RICH:
        table = Table(box=box.ROUNDED, border_style="blue", header_style="bold magenta")
        table.add_column("Mod Name", style="cyan"); table.add_column("Size", justify="right", style="green")
        for r in sorted(results, key=lambda x: x['size'], reverse=True):
            sz_str = format_size(r['size']) if r['size'] > 0 else "[yellow]Unknown[/]"
            table.add_row(r['name'], sz_str)
        table.add_section()
        table.add_row("[bold]TOTAL SIZE[/bold]", f"[bold yellow]{format_size(total_bytes)}[/bold yellow]")
        console.print(table)
    else:
        for r in sorted(results, key=lambda x: x['size'], reverse=True):
            print(f"{r['name']:<50} | {format_size(r['size'])}")
        print("-" * 60)
        print(f"{'TOTAL SIZE':<50} | {format_size(total_bytes)}")

if __name__ == "__main__":
    main()
