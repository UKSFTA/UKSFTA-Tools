#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import argparse
import urllib.request
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
    """Fetches mod title and dependencies from Steam Workshop."""
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    info = {"name": f"Mod {published_id}", "dependencies": []}
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as response:
            page = response.read().decode('utf-8')
            title_match = re.search(r'<div class="workshopItemTitle">(.*?)</div>', page)
            if title_match:
                info["name"] = html.unescape(title_match.group(1).strip())
            
            deps_section = re.search(r'id="RequiredItems">(.*?)</div>\s*</div>', page, re.DOTALL)
            if deps_section:
                items = re.findall(r'href=".*?id=(\d+)".*?>(.*?)</a>', deps_section.group(1), re.DOTALL)
                for dep_id, dep_html in items:
                    dep_name = re.sub(r'<[^>]+>', '', dep_html).strip()
                    info["dependencies"].append({
                        "id": dep_id.strip(),
                        "name": html.unescape(dep_name)
                    })
    except:
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
            
            is_ignored = ignore_block or any(x in clean_line.lower() for x in ["ignore=", "@ignore"])
            if is_ignored: ignored.add(mid)
            else: mods[mid] = name
                    
    return mods, ignored

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Modlist & Dependency Auditor")
    parser.add_argument("reference", help="The master/reference modlist (HTML or TXT)")
    parser.add_argument("source", help="The primary target modpack to audit")
    parser.add_argument("substitutes", nargs="*", help="Optional unit repositories or sources for fulfillment")
    parser.add_argument("--deep", action="store_true", help="Perform deep dependency scan on target mods")
    args = parser.parse_args()

    console = Console() if USE_RICH else None
    
    # Load Reference
    ref_mods, ref_ignored = extract_mods_with_ignores(args.reference)
    if not ref_mods:
        print(f"Error: No mods found in reference: {args.reference}")
        return

    # Load Source
    src_mods, src_ignored = extract_mods_with_ignores(args.source)
    
    # Load Substitutes
    all_sub_mods = {}
    all_sub_ignored = set()
    for s_path in args.substitutes:
        s_mods, s_ignored = extract_mods_with_ignores(s_path)
        all_sub_mods.update(s_mods)
        all_sub_ignored.update(s_ignored)

    # Combined Target State
    target_ids = set(src_mods.keys()) | set(all_sub_mods.keys())
    target_ignored = src_ignored | all_sub_ignored
    
    # --- PHASE 1: Audit ---
    missing_from_ref_raw = []
    for mid, name in ref_mods.items():
        if mid not in target_ids and mid not in target_ignored:
            missing_from_ref_raw.append({"id": mid, "name": name})

    missing_deps_raw = []
    if args.deep:
        to_scan = list(target_ids)
        if USE_RICH:
            with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
                task = progress.add_task(f"Auditing dependencies for {len(to_scan)} mods...", total=len(to_scan))
                def audit_dep(mid):
                    meta = fetch_workshop_data(mid)
                    found = []
                    for dep in meta['dependencies']:
                        did = dep['id']
                        if did not in target_ids and did not in target_ignored:
                            found.append({"id": did, "name": dep['name'], "parent": meta['name']})
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
                            missing_deps_raw.append({"id": did, "name": dep['name'], "parent": meta['name']})

    # --- PHASE 2: Resolution ---
    final_missing_ref = []
    final_missing_deps = []
    to_resolve = []
    for m in missing_from_ref_raw:
        if m['name'].startswith("Mod ") or m['name'].isdigit(): to_resolve.append(m)
        else: final_missing_ref.append(m)
    for m in missing_deps_raw:
        if m['name'].startswith("Mod ") or m['name'].isdigit(): to_resolve.append(m)
        else: final_missing_deps.append(m)

    if to_resolve:
        if USE_RICH:
            with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
                task = progress.add_task(f"Resolving names for {len(to_resolve)} mods...", total=len(to_resolve))
                def resolve(m):
                    res = fetch_workshop_data(m['id'])
                    m['name'] = res['name']
                    progress.advance(task)
                    return m
                with ThreadPoolExecutor(max_workers=10) as executor:
                    resolved_items = list(executor.map(resolve, to_resolve))
                    for m in resolved_items:
                        if 'parent' in m: final_missing_deps.append(m)
                        else: final_missing_ref.append(m)
        else:
            for m in to_resolve:
                m['name'] = fetch_workshop_data(m['id'])['name']
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
            console.print("[bold green]✅ PASS:[/] All reference mods and dependencies are accounted for across all sources.")
        else:
            if final_missing_ref:
                table = Table(title="❌ Missing Reference Mods", box=box.ROUNDED, border_style="red")
                table.add_column("Mod Name", style="bold red"); table.add_column("Workshop Link", style="blue underline")
                for m in sorted(final_missing_ref, key=lambda x: x['name']):
                    table.add_row(m['name'], f"https://steamcommunity.com/sharedfiles/filedetails/?id={m['id']}")
                console.print(table)

            if final_missing_deps:
                table = Table(title="⚠️ Missing Workshop Dependencies", box=box.ROUNDED, border_style="yellow")
                table.add_column("Required Mod", style="bold yellow"); table.add_column("Required By", style="dim"); table.add_column("Workshop Link", style="blue underline")
                for m in sorted(final_missing_deps, key=lambda x: x['name']):
                    table.add_row(m['name'], m['parent'], f"https://steamcommunity.com/sharedfiles/filedetails/?id={m['id']}")
                console.print(table)
    else:
        print(f"Reference: {args.reference}")
        print(f"Source:    {args.source}")
        if not final_missing_ref and not final_missing_deps: print("OK: All mods found.")
        else:
            for m in final_missing_ref: print(f"MISSING REF: {m['name']} (id={m['id']})")
            for m in final_missing_deps: print(f"MISSING DEP: {m['name']} (By {m['parent']}, id={m['id']})")

if __name__ == "__main__":
    main()
