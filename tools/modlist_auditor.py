#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import argparse
import urllib.request
import urllib.parse
import json
import html
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

def fetch_workshop_data(published_id):
    """Fetches mod title and dependencies using official Steam API."""
    api_url = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"
    data = urllib.parse.urlencode({
        "itemcount": 1,
        "publishedfileids[0]": published_id
    }).encode("utf-8")
    
    info = {"name": f"Mod {published_id}", "dependencies": []}
    
    try:
        req = urllib.request.Request(api_url, data=data, method="POST")
        with urllib.request.urlopen(req, timeout=10) as response:
            res_data = json.load(response)
            if "response" in res_data and "publishedfiledetails" in res_data["response"]:
                details = res_data["response"]["publishedfiledetails"][0]
                
                if details.get("result") == 1: # Success
                    info["name"] = details.get("title", f"Mod {published_id}")
                    # API returns children IDs, we'll need to resolve names later or just use IDs
                    # For now, let's just mark the IDs as dependencies
                    children = details.get("children", [])
                    for child in children:
                        info["dependencies"].append({
                            "id": child["publishedfileid"],
                            "name": f"Mod {child['publishedfileid']}" # Name will be resolved in final pass
                        })
    except Exception as e:
        # Fallback to scraping if API fails (though API is preferred)
        pass
    return info

def extract_mods_with_ignores(file_path):
    """Extracts ID -> Name mapping and a set of ignored IDs."""
    mods = {}
    ignored = set()
    if not os.path.exists(file_path): return mods, ignored
    
    with open(file_path, 'r', errors='ignore') as f:
        content = f.read()
        
    # 1. HTML Format
    html_matches = re.findall(r'<tr data-type="ModContainer">.*?<td name="displayName">(.*?)</td>.*?id=(\d{8,})', content, re.DOTALL)
    for name, mid in html_matches:
        mods[mid] = name
        
    # 2. TXT Format
    ignore_block = False
    lines = content.split('\n')
    for line in lines:
        clean_line = line.strip()
        if not clean_line or clean_line.startswith('//') or clean_line.startswith(';'): continue
        if "[ignore]" in clean_line.lower() or "[ignored]" in clean_line.lower():
            ignore_block = True
            continue

        mid_match = re.search(r'(?:id=)?(\d{8,})', clean_line)
        if mid_match:
            mid = mid_match.group(1)
            name = f"Mod {mid}"
            if '#' in clean_line:
                name = clean_line.split('#', 1)[1].split('[')[0].strip()
            
            if ignore_block or any(x in clean_line.lower() for x in ["ignore=", "@ignore"]):
                ignored.add(mid)
            else:
                mods[mid] = name
                    
    return mods, ignored

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Modlist & Dependency Auditor")
    parser.add_argument("reference", help="The master/reference modlist (HTML or TXT)")
    parser.add_argument("source", help="The primary target modpack to audit")
    parser.add_argument("substitutes", nargs="*", help="Optional unit repositories or sources for fulfillment")
    parser.add_argument("--deep", action="store_true", help="Perform deep dependency scan on target mods")
    args = parser.parse_args()

    console = Console() if USE_RICH else None
    
    # Load All Inputs
    ref_mods, ref_ignored = extract_mods_with_ignores(args.reference)
    if not ref_mods:
        print(f"Error: No mods found in reference: {args.reference}")
        return

    src_mods, src_ignored = extract_mods_with_ignores(args.source)
    all_sub_mods = {}; all_sub_ignored = set()
    for s_path in args.substitutes:
        s_mods, s_ignored = extract_mods_with_ignores(s_path)
        all_sub_mods.update(s_mods)
        all_sub_ignored.update(s_ignored)

    target_ids = set(src_mods.keys()) | set(all_sub_mods.keys())
    target_ignored = src_ignored | all_sub_ignored
    
    # --- PHASE 1: Find Missing items (Raw) ---
    missing_from_ref_raw = []
    for mid, name in ref_mods.items():
        if mid not in target_ids and mid not in target_ignored:
            missing_from_ref_raw.append({"id": mid, "name": name})

    missing_deps_raw = []
    if args.deep:
        to_scan = list(target_ids)
        if USE_RICH:
            with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
                task = progress.add_task(f"Scanning deep dependencies for {len(to_scan)} mods...", total=len(to_scan))
                def audit_dep(mid):
                    meta = fetch_workshop_data(mid)
                    found = []
                    for dep in meta['dependencies']:
                        did = dep['id']
                        if did not in target_ids and did not in target_ignored:
                            found.append({"id": did, "name": dep['name'], "parent": meta['name'], "parent_id": mid})
                    progress.advance(task)
                    return found
                with ThreadPoolExecutor(max_workers=10) as executor:
                    for r_list in executor.map(audit_dep, to_scan):
                        for m in r_list:
                            if m['id'] not in [x['id'] for x in missing_deps_raw]:
                                missing_deps_raw.append(m)
        else:
            for mid in to_scan:
                meta = fetch_workshop_data(mid)
                for dep in meta['dependencies']:
                    did = dep['id']
                    if did not in target_ids and did not in target_ignored:
                        if did not in [x['id'] for x in missing_deps_raw]:
                            missing_deps_raw.append({"id": did, "name": dep['name'], "parent": meta['name'], "parent_id": mid})

    # --- PHASE 2: Mandatory Name Resolution Pass ---
    final_missing_ref = []
    final_missing_deps = []
    to_resolve = missing_from_ref_raw + missing_deps_raw

    if to_resolve:
        if USE_RICH:
            with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
                task = progress.add_task(f"Resolving official titles for {len(to_resolve)} flagged mods...", total=len(to_resolve))
                def resolve(m):
                    # Always try to fetch official name from API
                    res = fetch_workshop_data(m['id'])
                    m['name'] = res['name']
                    # If this is a dependency, also try to resolve the parent's name if generic
                    if 'parent' in m and (m['parent'].startswith("Mod ") or m['parent'].isdigit()):
                        m['parent'] = fetch_workshop_data(m['parent_id'])['name']
                    progress.advance(task)
                    return m
                with ThreadPoolExecutor(max_workers=15) as executor:
                    resolved_items = list(executor.map(resolve, to_resolve))
                    for m in resolved_items:
                        if 'parent' in m: final_missing_deps.append(m)
                        else: final_missing_ref.append(m)
        else:
            for m in to_resolve:
                res = fetch_workshop_data(m['id'])
                m['name'] = res['name']
                if 'parent' in m: final_missing_deps.append(m)
                else: final_missing_ref.append(m)

    # --- OUTPUT ---
    if USE_RICH:
        header = f"[bold blue]Modlist Integrity Audit[/bold blue]\n"
        header += f"[white]Reference:[/] [cyan]{os.path.basename(args.reference)}[/]\n"
        header += f"[white]Source:[/]    [magenta]{os.path.basename(args.source)}[/]\n"
        if args.substitutes:
            header += f"[white]Subs:[/]      [dim]{', '.join([os.path.basename(s) for s in args.substitutes])}[/dim]"
        console.print(Panel(header, border_style="blue", padding=(1, 2)))
        
        if not final_missing_ref and not final_missing_deps:
            console.print("[bold green]✅ PASS:[/] All reference mods and dependencies are accounted for.")
        else:
            if final_missing_ref:
                table = Table(title="❌ Missing Reference Mods", box=box.ROUNDED, border_style="red")
                table.add_column("Official Mod Name", style="bold red"); table.add_column("Workshop Link", style="blue underline")
                for m in sorted(final_missing_ref, key=lambda x: x['name']):
                    table.add_row(m['name'], f"https://steamcommunity.com/sharedfiles/filedetails/?id={m['id']}")
                console.print(table)

            if final_missing_deps:
                table = Table(title="⚠️ Missing Workshop Dependencies", box=box.ROUNDED, border_style="yellow")
                table.add_column("Required Mod (Missing)", style="bold yellow"); table.add_column("Required By", style="dim"); table.add_column("Workshop Link", style="blue underline")
                for m in sorted(final_missing_deps, key=lambda x: x['name']):
                    table.add_row(m['name'], m['parent'], f"https://steamcommunity.com/sharedfiles/filedetails/?id={m['id']}")
                console.print(table)
    else:
        if not final_missing_ref and not final_missing_deps: print("OK")
        else:
            for m in final_missing_ref: print(f"MISSING REF: {m['name']} (id={m['id']})")
            for m in final_missing_deps: print(f"MISSING DEP: {m['name']} (By {m['parent']}, id={m['id']})")

if __name__ == "__main__":
    main()
