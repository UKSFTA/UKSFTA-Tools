#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import json
import argparse
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor
from mod_classifier import classify_mod

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

def get_mod_data_from_html(file_path):
    """Extracts Name and ID from Arma 3 HTML preset."""
    mods = {} # ID -> Name
    if not os.path.exists(file_path): return mods
    
    with open(file_path, 'r', errors='ignore') as f:
        content = f.read()
        
    # Arma 3 HTML Presets use a specific table structure
    # We look for the display name and the workshop link
    matches = re.findall(r'<tr data-type="ModContainer">.*?<td name="displayName">(.*?)</td>.*?id=(\d{8,})', content, re.DOTALL)
    for name, mid in matches:
        mods[mid] = name
        
    # Fallback for mod_sources.txt or other raw formats
    if not mods:
        raw_ids = re.findall(r'(?:id=)?(\d{8,})', content)
        for mid in raw_ids:
            mods[mid] = f"Mod {mid}"
            
    return mods

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Modlist Classifier")
    parser.add_argument("file", nargs="?", default="mod_sources.txt", help="Path to modlist HTML or mod_sources.txt")
    parser.add_argument("--json", action="store_true", help="Output results as JSON")
    args = parser.parse_args()

    input_file = args.file
    if not os.path.exists(input_file):
        print(f"Error: {input_file} not found.")
        return

    mod_data = get_mod_data_from_html(input_file)
    if not mod_data:
        print("No Workshop IDs found in file.")
        return

    console = Console() if USE_RICH else None
    results = []

    if USE_RICH:
        with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
            task = progress.add_task(f"Classifying {len(mod_data)} mods...", total=len(mod_data))
            with ThreadPoolExecutor(max_workers=10) as executor:
                future_to_id = {executor.submit(classify_mod, mid): mid for mid in mod_data.keys()}
                for future in future_to_id:
                    res = future.result()
                    if res:
                        # Override title with local name from HTML if available
                        mid = future_to_id[future]
                        if mod_data[mid] != f"Mod {mid}":
                            res['title'] = mod_data[mid]
                        results.append(res)
                    progress.advance(task)
    else:
        print(f"Classifying {len(mod_data)} mods...")
        for mid in mod_data.keys():
            res = classify_mod(mid)
            if res: results.append(res)

    if args.json:
        print(json.dumps(results, indent=2))
        return

    if USE_RICH:
        table = Table(title=f"Modlist Side-Requirement Audit: {os.path.basename(input_file)}", box=box.ROUNDED, border_style="blue")
        table.add_column("Mod Name", style="bold white")
        table.add_column("Verdict", justify="center")
        table.add_column("Confidence", justify="right")
        table.add_column("Evidence Hint", style="dim")

        counts = {"Both": 0, "Client": 0, "Server": 0, "Indecisive": 0}
        for r in sorted(results, key=lambda x: x['result']):
            color = "yellow" if r['result'] == "Both" else ("green" if r['result'] == "Client" else "magenta")
            if r['result'] == "Indecisive": color = "white"
            counts[r['result']] += 1
            
            reason = r['evidence'][0]['text'] if r['evidence'] else "N/A"
            if len(reason) > 50: reason = reason[:47] + "..."
            table.add_row(r['title'], f"[{color}]{r['result']}[/]", f"{r['confidence']}%", reason)
        
        console.print(table)
        summary = f"[bold yellow]Both:[/] {counts['Both']} | [bold green]Client:[/] {counts['Client']} | [bold magenta]Server:[/] {counts['Server']} | [bold white]Unknown:[/] {counts['Indecisive']}"
        console.print(Panel(summary, title="Modlist Composition"))
    else:
        for r in sorted(results, key=lambda x: x['result']):
            print(f"{r['result']:<12} | {r['confidence']:>5}% | {r['title']}")

if __name__ == "__main__":
    main()
