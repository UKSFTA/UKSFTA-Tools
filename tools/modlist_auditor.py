#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import argparse
from pathlib import Path

# Try to import rich for high-fidelity CLI output
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    from rich.panel import Panel
    USE_RICH = True
except ImportError:
    USE_RICH = False

def extract_mods(file_path):
    """Extracts ID -> Name mapping from HTML or TXT files."""
    mods = {}
    if not os.path.exists(file_path): return mods
    
    with open(file_path, 'r', errors='ignore') as f:
        content = f.read()
        
    # 1. Try HTML Format (Arma Preset)
    html_matches = re.findall(r'<tr data-type="ModContainer">.*?<td name="displayName">(.*?)</td>.*?id=(\d{8,})', content, re.DOTALL)
    for name, mid in html_matches:
        mods[mid] = name
        
    # 2. Try TXT Format (mod_sources.txt with # Comments)
    if not mods:
        lines = content.split('\n')
        for line in lines:
            line = line.strip()
            if not line or line.startswith('//') or line.startswith(';'): continue
            
            if '#' in line:
                id_part, name_part = line.split('#', 1)
                mid_match = re.search(r'(\d{8,})', id_part)
                if mid_match:
                    mods[mid_match.group(1)] = name_part.strip()
            else:
                mid_match = re.search(r'(\d{8,})', line)
                if mid_match:
                    mods[mid_match.group(1)] = f"Mod {mid_match.group(1)}"
                    
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

    missing = []
    for mid, name in ref_mods.items():
        if mid not in target_ids:
            missing.append({"id": mid, "name": name, "url": f"https://steamcommunity.com/sharedfiles/filedetails/?id={mid}"})

    if USE_RICH:
        title = f"[bold blue]Modlist Integrity Audit[/bold blue]\n[dim]Reference: {os.path.basename(args.reference)}[/dim]"
        console.print(Panel.fit(title, border_style="blue"))
        
        if not missing:
            console.print("[bold green]✅ PASS:[/] All mods from reference are present in targets.")
        else:
            table = Table(title=f"⚠️ {len(missing)} Missing Dependencies Found", box=box.ROUNDED, border_style="red")
            table.add_column("Missing Mod Name", style="bold red")
            table.add_column("Workshop Link", style="blue underline")
            
            for m in missing:
                table.add_row(m['name'], m['url'])
            
            console.print(table)
            console.print(f"\n[red]Failure:[/] The target modlists are missing {len(missing)} mods required by the reference.")
    else:
        if not missing:
            print("OK: All reference mods found in targets.")
        else:
            print(f"FAILED: {len(missing)} mods missing from targets.")
            for m in missing:
                print(f" - {m['name']} ({m['url']})")

if __name__ == "__main__":
    main()
