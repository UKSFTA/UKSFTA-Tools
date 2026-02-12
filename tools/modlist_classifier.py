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

def get_mod_ids_from_file(file_path):
    ids = set()
    if not os.path.exists(file_path): return ids
    with open(file_path, 'r', errors='ignore') as f:
        content = f.read()
        matches = re.findall(r'(?:id=)?(\d{8,})', content)
        for m in matches: ids.add(m)
    return ids

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Modlist Classifier")
    parser.add_argument("file", nargs="?", default="mod_sources.txt", help="Path to modlist file")
    parser.add_argument("--json", action="store_true", help="Output results as JSON")
    args = parser.parse_args()

    input_file = args.file
    if not os.path.exists(input_file):
        print(f"Error: {input_file} not found.")
        return

    mod_ids = get_mod_ids_from_file(input_file)
    if not mod_ids:
        print("No Workshop IDs found.")
        return

    console = Console() if USE_RICH else None
    results = []

    if USE_RICH:
        with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
            task = progress.add_task(f"Classifying {len(mod_ids)} mods...", total=len(mod_ids))
            
            with ThreadPoolExecutor(max_workers=10) as executor:
                future_to_id = {executor.submit(classify_mod, mid): mid for mid in mod_ids}
                for future in future_to_id:
                    res = future.result()
                    if res: results.append(res)
                    progress.advance(task)
    else:
        print(f"Classifying {len(mod_ids)} mods...")
        for mid in mod_ids:
            res = classify_mod(mid)
            if res: results.append(res)

    if args.json:
        print(json.dumps(results, indent=2))
        return

    if USE_RICH:
        table = Table(title=f"Modlist Side-Requirement Audit: {input_file}", box=box.ROUNDED, border_style="blue")
        table.add_column("Mod Name", style="bold white")
        table.add_column("Verdict", justify="center")
        table.add_column("Confidence", justify="right")
        table.add_column("Primary Reason", style="dim")

        # Categorize for summary
        counts = {"Both": 0, "Client": 0, "Server": 0, "Indecisive": 0}

        for r in sorted(results, key=lambda x: x['result']):
            color = "yellow" if r['result'] == "Both" else ("green" if r['result'] == "Client" else "magenta")
            if r['result'] == "Indecisive": color = "white"
            counts[r['result']] += 1
            
            reason = r['evidence'][0]['text'] if r['evidence'] else "No markers found"
            if len(reason) > 60: reason = reason[:57] + "..."
            
            table.add_row(r['title'], f"[{color}]{r['result']}[/]", f"{r['confidence']}%", reason)
        
        console.print(table)
        
        summary = f"[bold yellow]Both:[/] {counts['Both']} | [bold green]Client:[/] {counts['Client']} | [bold magenta]Server:[/] {counts['Server']} | [bold white]Unknown:[/] {counts['Indecisive']}"
        console.print(Panel(summary, title="Summary"))
    else:
        for r in results:
            print(f"{r['result']:<12} | {r['confidence']:>5}% | {r['title']}")

if __name__ == "__main__":
    main()
