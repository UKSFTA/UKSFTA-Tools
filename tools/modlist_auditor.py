#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import argparse
import urllib.request
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

# Try to import rich for high-fidelity CLI output
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    from rich.panel import Panel
    from rich.progress import Progress, SpinnerColumn, TextColumn
    USE_RICH = True
except ImportError:
    USE_RICH = False

def fetch_mod_name(published_id):
    """Fetches mod title from Steam Workshop."""
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=5) as response:
            html_content = response.read().decode('utf-8')
            title_match = re.search(r'<div class="workshopItemTitle">(.*?)</div>', html_content)
            if title_match:
                return title_match.group(1).strip()
    except:
        pass
    return f"Mod {published_id}"

def extract_mods(file_path):
    """Extracts ID -> Name mapping from HTML or TXT files."""
    mods = {}
    if not os.path.exists(file_path): return mods
    
    with open(file_path, 'r', errors='ignore') as f:
        content = f.read()
        
    # 1. Try HTML Format (Arma Preset)
    # We look for the display name and the workshop link ID
    html_matches = re.findall(r'<tr data-type="ModContainer">.*?<td name="displayName">(.*?)</td>.*?id=(\d{8,})', content, re.DOTALL)
    for name, mid in html_matches:
        mods[mid] = name
        
    # 2. Try TXT Format (mod_sources.txt)
    # Fallback/Supplemental: Find IDs and try to get names if not already found
    lines = content.split('\n')
    for line in lines:
        line = line.strip()
        if not line or line.startswith('//') or line.startswith(';'): continue
        
        # Format: ID # Name
        if '#' in line:
            id_part, name_part = line.split('#', 1)
            mid_match = re.search(r'(\d{8,})', id_part)
            if mid_match:
                mid = mid_match.group(1)
                if mid not in mods: mods[mid] = name_part.strip()
        else:
            # Format: raw ID or URL
            mid_match = re.search(r'(?:id=)?(\d{8,})', line)
            if mid_match:
                mid = mid_match.group(1)
                if mid not in mods: mods[mid] = f"Mod {mid}"
                    
    return mods

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Modlist Integrity Auditor")
    parser.add_argument("reference", help="The master/reference modlist (HTML or TXT)")
    parser.add_argument("targets", nargs="+", help="One or more modlists/sources to check against the reference")
    args = parser.parse_args()

    console = Console() if USE_RICH else None
    
    # Load Reference
    ref_mods = extract_mods(args.reference)
    if not ref_mods:
        print(f"Error: No mods found in reference file: {args.reference}")
        return

    # Load all targets into one master set
    target_ids = set()
    for t_path in args.targets:
        t_mods = extract_mods(t_path)
        target_ids.update(t_mods.keys())

    missing_raw = []
    for mid, name in ref_mods.items():
        if mid not in target_ids:
            missing_raw.append({"id": mid, "name": name})

    if not missing_raw:
        if USE_RICH:
            console.print(Panel("[bold green]✅ PASS:[/] All mods from reference are present in targets.", title="Modlist Integrity Audit", border_style="green"))
        else:
            print("OK: All reference mods found in targets.")
        return

    # Resolve names for missing mods if they are generic
    missing = []
    if USE_RICH:
        with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
            task = progress.add_task(f"Resolving names for {len(missing_raw)} missing mods...", total=len(missing_raw))
            
            def resolve(m):
                name = m['name']
                if name.startswith("Mod "):
                    name = fetch_mod_name(m['id'])
                progress.advance(task)
                return {"id": m['id'], "name": name, "url": f"https://steamcommunity.com/sharedfiles/filedetails/?id={m['id']}"}

            with ThreadPoolExecutor(max_workers=10) as executor:
                missing = list(executor.map(resolve, missing_raw))
    else:
        print(f"Resolving names for {len(missing_raw)} missing mods...")
        for m in missing_raw:
            name = m['name']
            if name.startswith("Mod "): name = fetch_mod_name(m['id'])
            missing.append({"id": m['id'], "name": name, "url": f"https://steamcommunity.com/sharedfiles/filedetails/?id={m['id']}"})

    if USE_RICH:
        title = f"[bold blue]Modlist Integrity Audit[/bold blue]\n[dim]Reference: {os.path.basename(args.reference)}[/dim]"
        console.print(Panel.fit(title, border_style="blue"))
        
        table = Table(title=f"⚠️ {len(missing)} Missing Dependencies Found", box=box.ROUNDED, border_style="red")
        table.add_column("Missing Mod Name", style="bold red")
        table.add_column("Workshop Link", style="blue underline")
        
        for m in sorted(missing, key=lambda x: x['name']):
            table.add_row(m['name'], m['url'])
        
        console.print(table)
        console.print(f"\n[red]Failure:[/] The target modlists are missing {len(missing)} mods required by the reference.")
    else:
        print(f"FAILED: {len(missing)} mods missing from targets.")
        for m in sorted(missing, key=lambda x: x['name']):
            print(f" - {m['name']} ({m['url']})")

if __name__ == "__main__":
    main()
