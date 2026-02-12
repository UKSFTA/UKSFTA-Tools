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
import time
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

def fetch_workshop_name(published_id):
    """Uses Steam API for name resolution."""
    api_url = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"
    data = urllib.parse.urlencode({"itemcount": 1, "publishedfileids[0]": published_id}).encode("utf-8")
    try:
        req = urllib.request.Request(api_url, data=data, method="POST")
        with urllib.request.urlopen(req, timeout=10) as response:
            res_data = json.load(response)
            if "response" in res_data and "publishedfiledetails" in res_data["response"]:
                details = res_data["response"]["publishedfiledetails"][0]
                if details.get("result") == 1:
                    return details.get("title", f"Mod {published_id}")
    except: pass
    return f"Mod {published_id}"

def fetch_workshop_dependencies(published_id):
    """Uses HTML Scraping for dependencies (Required Items + Description scanning)."""
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    deps = set() # Set of IDs
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36'})
        with urllib.request.urlopen(req, timeout=10) as response:
            page = response.read().decode('utf-8')
            
            # 1. Official Required Items
            deps_section = re.search(r'id="RequiredItems">(.*?)</div>\s*</div>', page, re.DOTALL)
            if deps_section:
                items = re.findall(r'href=".*?id=(\d+)"', deps_section.group(1))
                for did in items: deps.add(did)
            
            # 2. Description Scanning (Catch hidden dependencies)
            desc_match = re.search(r'<div class="workshopItemDescription" id="highlightContent">(.*?)</div>', page, re.DOTALL)
            if desc_match:
                # Find all Steam Workshop links or raw IDs in the description
                desc_text = desc_match.group(1)
                links = re.findall(r'(?:id=)?(\d{8,})', desc_text)
                for did in links:
                    if did != published_id: deps.add(did)
    except: pass
    return [{"id": d, "name": f"Mod {d}"} for d in deps]

def extract_mods_with_ignores(file_path):
    """Extracts ID -> Name mapping and a set of ignored IDs. Also loads mods.lock if present."""
    mods = {}
    ignored = set()
    path = Path(file_path)
    if not path.exists(): return mods, ignored
    
    # 1. Load the primary file
    content = path.read_text(errors='ignore')
    html_matches = re.findall(r'<tr data-type="ModContainer">.*?<td name="displayName">(.*?)</td>.*?id=(\d{8,})', content, re.DOTALL)
    for name, mid in html_matches: mods[mid] = name
    
    if not mods:
        ignore_block = False
        lines = content.split('\n')
        for line in lines:
            clean_line = line.strip()
            if not clean_line or clean_line.startswith('//') or clean_line.startswith(';'): continue
            if "[ignore]" in clean_line.lower() or "[ignored]" in clean_line.lower():
                ignore_block = True; continue
            mid_match = re.search(r'(?:id=)?(\d{8,})', clean_line)
            if mid_match:
                mid = mid_match.group(1); name = f"Mod {mid}"
                if '#' in clean_line: name = clean_line.split('#', 1)[1].split('[')[0].strip()
                if ignore_block or any(x in clean_line.lower() for x in ["ignore=", "@ignore"]): ignored.add(mid)
                else: mods[mid] = name

    # 2. Check for mods.lock next to the file (Automated transitive fulfillment)
    lock_path = path.parent / "mods.lock"
    if lock_path.exists():
        try:
            lock_data = json.loads(lock_path.read_text())
            for mid, info in lock_data.get("mods", {}).items():
                if mid not in mods and mid not in ignored:
                    mods[mid] = info.get("name", f"Mod {mid}")
        except: pass
        
    return mods, ignored

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Modlist & Dependency Auditor")
    parser.add_argument("reference", help="The master/reference modlist (HTML or TXT)")
    parser.add_argument("source", help="The primary target modpack to audit")
    parser.add_argument("substitutes", nargs="*", help="Optional unit repositories or sources for fulfillment")
    parser.add_argument("--deep", action="store_true", help="Perform deep dependency scan on target mods")
    args = parser.parse_args()
    console = Console() if USE_RICH else None
    
    # Load Inputs
    ref_mods, ref_ignored = extract_mods_with_ignores(args.reference)
    src_mods, src_ignored = extract_mods_with_ignores(args.source)
    all_sub_mods = {}; all_sub_ignored = set()
    for s_path in args.substitutes:
        s_mods, s_ignored = extract_mods_with_ignores(s_path)
        all_sub_mods.update(s_mods); all_sub_ignored.update(s_ignored)

    explicit_target_ids = set(src_mods.keys()) | set(all_sub_mods.keys())
    all_target_ignored = src_ignored | all_sub_ignored
    
    # --- PHASE 1: Dependency Resolution ---
    resolved_target_ids = set(explicit_target_ids)
    missing_deps_raw = []
    
    if args.deep:
        to_scan = list(explicit_target_ids)
        processed = set()
        if USE_RICH:
            with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
                task = progress.add_task(f"Scanning deep dependencies for {len(to_scan)} mods...", total=None)
                while to_scan:
                    mid = to_scan.pop(0)
                    if mid in processed: continue
                    processed.add(mid)
                    parent_name = src_mods.get(mid) or all_sub_mods.get(mid) or f"Mod {mid}"
                    deps = fetch_workshop_dependencies(mid)
                    for dep in deps:
                        did = dep['id']
                        if did not in all_target_ignored:
                            resolved_target_ids.add(did)
                            if did not in explicit_target_ids and did not in ref_mods:
                                if did not in [x['id'] for x in missing_deps_raw]:
                                    missing_deps_raw.append({"id": did, "name": dep['name'], "parent": parent_name, "parent_id": mid})
                        if did not in processed: to_scan.append(did)
                    progress.advance(task)
        else:
            while to_scan:
                mid = to_scan.pop(0)
                if mid in processed: continue
                processed.add(mid)
                deps = fetch_workshop_dependencies(mid)
                for dep in deps:
                    did = dep['id']
                    if did not in all_target_ignored:
                        resolved_target_ids.add(did)
                        if did not in explicit_target_ids and did not in ref_mods and did not in [x['id'] for x in missing_deps_raw]:
                            missing_deps_raw.append({"id": did, "name": dep['name'], "parent": f"Mod {mid}", "parent_id": mid})
                    if did not in processed: to_scan.append(did)

    # --- PHASE 2: Check Missing Reference Mods ---
    missing_from_ref_raw = []
    for mid, name in ref_mods.items():
        if mid not in resolved_target_ids:
            missing_from_ref_raw.append({"id": mid, "name": name})

    # --- PHASE 3: Mandatory Name Resolution ---
    final_missing_ref = []
    final_missing_deps = []
    to_resolve = missing_from_ref_raw + missing_deps_raw
    if to_resolve:
        if USE_RICH:
            with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
                task = progress.add_task(f"Resolving official titles for {len(to_resolve)} flagged mods...", total=len(to_resolve))
                def resolve(m):
                    res_info = fetch_workshop_name(m['id'])
                    m['name'] = res_info
                    if 'parent' in m and (m['parent'].startswith("Mod ") or m['parent'].isdigit()):
                        m['parent'] = fetch_workshop_name(m['parent_id'])
                    progress.advance(task); return m
                with ThreadPoolExecutor(max_workers=15) as executor:
                    resolved_items = list(executor.map(resolve, to_resolve))
                    for m in resolved_items:
                        if 'parent' in m: final_missing_deps.append(m)
                        else: final_missing_ref.append(m)
        else:
            for m in to_resolve:
                m['name'] = fetch_workshop_name(m['id'])
                if 'parent' in m and (m['parent'].startswith("Mod ") or m['parent'].isdigit()):
                    m['parent'] = fetch_workshop_name(m['parent_id'])
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
            console.print("[bold green]✅ PASS:[/] All reference mods and deep dependencies are accounted for.")
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
            for m in final_missing_ref: print(f"MISSING REF: {m['name']} (https://steamcommunity.com/sharedfiles/filedetails/?id={m['id']})")
            for m in final_missing_deps: print(f"MISSING DEP: {m['name']} (Required by {m['parent']}, id={m['id']})")

if __name__ == "__main__":
    main()
