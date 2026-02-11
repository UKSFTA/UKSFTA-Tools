#!/usr/bin/env python3
import requests
import json
import os
import sys
import re
import urllib.request
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

# Steam Web API endpoint for Workshop details
STEAM_API_URL = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"

def scrape_workshop_details(published_id):
    """Fallback: Scrape the HTML page for unlisted items."""
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    details = {"size": "N/A", "posted": "N/A", "updated": "N/A"}
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36'})
        with urllib.request.urlopen(req, timeout=10) as response:
            html_content = response.read().decode('utf-8')
            
            # 1. Look for data-timestamp attribute (Primary Update Time)
            ts_match = re.search(r'data-timestamp="(\d+)"', html_content)
            if ts_match:
                details["updated"] = datetime.fromtimestamp(int(ts_match.group(1))).strftime('%d %b %Y')
            
            # 2. Extract Sidebar Stats
            all_stats = re.findall(r'<div class="detailsStatRight">(.*?)</div>', html_content)
            if len(all_stats) >= 3:
                details["size"] = all_stats[0].strip()
                details["posted"] = all_stats[1].strip()
                if details["updated"] == "N/A":
                    details["updated"] = all_stats[2].strip()
            elif len(all_stats) == 2:
                details["size"] = all_stats[0].strip()
                details["posted"] = all_stats[1].strip()
                
            return details
    except:
        return details

def get_workshop_details(published_ids):
    if not published_ids: return []
    data = {"itemcount": len(published_ids)}
    for i, pid in enumerate(published_ids): data[f"publishedfileids[{i}]"] = pid
    try:
        response = requests.post(STEAM_API_URL, data=data)
        response.raise_for_status()
        return response.json().get("response", {}).get("publishedfiledetails", [])
    except: return []

def main():
    console = Console() if USE_RICH else None
    
    if USE_RICH:
        console.print(Panel.fit("🔍 [bold blue]UKSFTA Workshop Intelligence[/bold blue]", border_style="blue"))
    else:
        print("\n 🔍 UKSFTA Workshop Intelligence\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n")

    # 1. Discover Projects
    projects = []
    current_dir = Path(__file__).parent.parent
    parent_dir = current_dir.parent
    
    for d in parent_dir.iterdir():
        toml_path = d / ".hemtt" / "project.toml"
        if toml_path.exists():
            with open(toml_path, "r") as f:
                content = f.read()
                match = re.search(r'workshop_id\s*=\s*"(.*?)"', content)
                if match and match.group(1) != "0":
                    projects.append({"name": d.name, "id": match.group(1)})

    if not projects:
        if USE_RICH: console.print("[yellow]! No projects with Workshop IDs found.[/yellow]")
        else: print(" ! No projects with Workshop IDs found.")
        return

    # 2. Query Data
    ids = [p["id"] for p in projects]
    details = get_workshop_details(ids)
    results = []
    
    for detail in details:
        pid = detail.get("publishedfileid")
        proj = next((p for p in projects if p["id"] == pid), None)
        if not proj: continue
        
        res_code = detail.get("result")
        status = "Public" if res_code == 1 else "Unlisted"
        
        if res_code == 1:
            u_ts = detail.get("time_updated", 0)
            p_ts = detail.get("time_created", 0)
            updated = datetime.fromtimestamp(u_ts).strftime('%d %b %Y') if u_ts else "Never"
            posted = datetime.fromtimestamp(p_ts).strftime('%d %b %Y') if p_ts else "Unknown"
            size = f"{int(detail.get('file_size', 0)) / (1024**1024):.2f} MB"
        else:
            scraped = scrape_workshop_details(pid)
            updated, posted, size = scraped["updated"], scraped["posted"], scraped["size"]
            if updated == "N/A" and posted == "N/A": status = "Private"

        results.append({
            "name": proj["name"], "status": status, "size": size, 
            "posted": posted, "updated": updated, 
            "link": f"https://steamcommunity.com/sharedfiles/filedetails/?id={pid}"
        })

    # 3. Render
    if USE_RICH:
        table = Table(box=box.ROUNDED, header_style="bold magenta", border_style="blue")
        table.add_column("Project", style="cyan")
        table.add_column("Status", justify="center")
        table.add_column("Size", justify="right", style="green")
        table.add_column("Posted", style="dim")
        table.add_column("Last Updated", style="bold yellow")
        table.add_column("Workshop Link", style="blue underline")

        for r in sorted(results, key=lambda x: x['name']):
            icon = "✅" if r["status"] == "Public" else ("🔗" if r["status"] == "Unlisted" else "🔒")
            table.add_row(r['name'], f"{icon} {r['status']}", r['size'], r['posted'], r['updated'], r['link'])
        console.print(table)
    else:
        # Fallback for non-rich
        print(f"{'Project':<18} | {'Status':<10} | {'Size':<10} | {'Updated'}")
        for r in sorted(results, key=lambda x: x['name']):
            print(f"{r['name']:<18} | {r['status']:<10} | {r['size']:<10} | {r['updated']}")

if __name__ == "__main__":
    main()
