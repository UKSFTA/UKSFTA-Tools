#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import argparse
import os
import subprocess
import re
import sys
import json
import shutil
import urllib.request
from pathlib import Path
from datetime import datetime
from concurrent.futures import ThreadPoolExecutor

# Soft-import rich for CI environments
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    from rich.panel import Panel
    from rich.text import Text
    from rich.progress import Progress, SpinnerColumn, TextColumn
    USE_RICH = True
except ImportError:
    USE_RICH = False
    class Table:
        def __init__(self, **kwargs): self.rows = []
        def add_column(self, *args, **kwargs): pass
        def add_row(self, *args): self.rows.append(args)
    class Console:
        def __init__(self, *args, **kwargs): pass
        def print(self, obj):
            if hasattr(obj, 'rows'):
                for r in obj.rows: print(" | ".join(map(str, r)))
            else: print(obj)
    class box: ROUNDED = None
    class Panel:
        @staticmethod
        def fit(text, title=None, **kwargs): return f"--- {title} ---\n{text}"

def is_project(path):
    return (path / ".hemtt" / "project.toml").exists() or (path / "mod_sources.txt").exists()

def get_projects():
    cwd = Path.cwd()
    current = cwd
    while current != current.parent:
        if is_project(current) and current.name.startswith("UKSFTA-"): return [current]
        current = current.parent
    parent_dir = Path(__file__).parent.parent.parent.resolve()
    projects = []
    if parent_dir.exists():
        for d in parent_dir.iterdir():
            if d.is_dir() and d.name.startswith("UKSFTA-") and is_project(d): projects.append(d)
    return sorted(projects)

def print_banner(console):
    version = "Unknown"
    v_path = Path(__file__).parent.parent / "VERSION"
    if v_path.exists(): version = v_path.read_text().strip()
    banner = Text.assemble(("\n [!] ", "bold blue"), ("UKSF TASKFORCE ALPHA ", "bold white"), ("| ", "dim"), ("PLATINUM DEVOPS SUITE ", "bold cyan"), (f"v{version}", "bold yellow"), ("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n", "dim blue"))
    console.print(banner)

def cmd_help(console):
    print_banner(console)
    ws_table = Table(title="[Workspace Operations]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold cyan")
    ws_table.add_row("[bold cyan]status   [/]", "[dim]Show git status summary[/]")
    ws_table.add_row("[bold cyan]update   [/]", "[dim]Propagate latest UKSFTA-Tools[/]")
    ws_table.add_row("[bold cyan]lint     [/]", "[dim]Full Quality Suite (MD, JSON, Config, SQF)[/]")
    ws_table.add_row("[bold cyan]build    [/]", "[dim]Execute HEMTT build[/]")
    
    audit_table = Table(title="[Assurance & Quality]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold yellow")
    audit_table.add_row("[bold cyan]audit-security   [/]", "[dim]Scan for leaked tokens/keys[/]")
    audit_table.add_row("[bold cyan]audit-performance[/]", "[dim]Scan for texture bottlenecks[/]")
    
    intel_table = Table(title="[Intelligence & COP]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold magenta")
    intel_table.add_row("[bold cyan]gh-runs         [/]", "[dim]Monitor GitHub Actions[/]")
    intel_table.add_row("[bold cyan]harvest-terrain [/]", "[dim]Ingest terrain PNG into COP tiles[/]")
    
    console.print(ws_table); console.print(audit_table); console.print(intel_table)

def cmd_lint(args):
    console = Console(force_terminal=True); print_banner(console)
    subprocess.run(["npx", "--yes", "markdownlint-cli2", "**/*.md", "--config", ".github/linters/.markdownlint.json"])
    for p in get_projects():
        print(f"Auditing Project: {p.name}")
        subprocess.run([sys.executable, "tools/config_style_checker.py", str(p)])
        subprocess.run([sys.executable, "tools/sqf_validator.py", str(p)])

def cmd_gh_runs(args):
    console = Console(force_terminal=True); print_banner(console)
    projects = get_projects(); workflow_names = set(); all_stats = []
    with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
        task = progress.add_task("[cyan]Gathering pipeline intelligence...", total=len(projects))
        for p in projects:
            stats = {"project": p.name, "workflows": {}, "latest_age": "-"}
            try:
                res = subprocess.run(["gh", "run", "list", "--limit", "10", "--json", "workflowName,conclusion,status,createdAt"], cwd=p, capture_output=True, text=True)
                if res.returncode == 0:
                    runs = json.loads(res.stdout)
                    for run in runs:
                        wf = run['workflowName']; workflow_names.add(wf)
                        if wf not in stats["workflows"]:
                            stats["workflows"][wf] = {"status": run['status'], "conclusion": run['conclusion']}
                            if stats["latest_age"] == "-":
                                created = datetime.fromisoformat(run['createdAt'].replace('Z', '+00:00'))
                                diff = datetime.now(created.tzinfo) - created
                                stats["latest_age"] = f"{diff.days}d" if diff.days > 0 else f"{diff.seconds // 3600}h"
            except: pass
            all_stats.append(stats); progress.advance(task)
    if not workflow_names: console.print("[yellow]! No pipeline data found.[/yellow]"); return
    sorted_workflows = sorted(list(workflow_names))
    table = Table(title=f"Unit Pipeline Matrix", box=box.ROUNDED, border_style="blue")
    table.add_column("Project", style="cyan")
    for wf in sorted_workflows: table.add_column(wf.replace(".yml", "").capitalize(), justify="center")
    table.add_column("Age", justify="right")
    for s in all_stats:
        row_icons = []
        for wf in sorted_workflows:
            w_stat = s["workflows"].get(wf, {"status": "none", "conclusion": "none"})
            if w_stat["status"] == "none": icon = "[dim]-[/dim]"
            elif w_stat["status"] != "completed": icon = "[bold yellow]...[/]"
            elif w_stat["conclusion"] == "success": icon = "[bold green]PASS[/]"
            else: icon = "[bold red]FAIL[/]"
            row_icons.append(icon)
        table.add_row(s["project"], *row_icons, s["latest_age"])
    console.print(table)

def main():
    parser = argparse.ArgumentParser(description="UKSF Taskforce Alpha Manager", add_help=False)
    parser.add_argument("--json", action="store_true")
    subparsers = parser.add_subparsers(dest="command")
    
    # Core registered commands
    for cmd in ["status", "update", "gh-runs", "generate-catalog", "build", "release", "audit-security", "audit-performance", "audit-assets", "audit-signatures", "audit-keys", "audit-deps", "audit-strings", "audit-mission", "audit-updates", "apply-updates", "generate-manifest", "generate-preset", "generate-report", "generate-vscode", "generate-changelog", "fix-syntax", "check-env", "self-update"]:
        subparsers.add_parser(cmd)
    
    p_lint = subparsers.add_parser("lint")
    p_lint.add_argument("--fix", action="store_true")
    
    p_harvest = subparsers.add_parser("harvest-terrain")
    p_harvest.add_argument("image"); p_harvest.add_argument("name")
    
    args = parser.parse_args(); console = Console(force_terminal=True)
    cmds = {
        "gh-runs": cmd_gh_runs,
        "lint": cmd_lint,
        "update": lambda a: [subprocess.run([sys.executable, str(p/".uksf_tools/setup.py")], cwd=p) for p in get_projects()],
        "status": lambda a: [print(f"Project: {p.name}") for p in get_projects()],
        "build": lambda a: [subprocess.run(["bash", "build.sh", "build"], cwd=p) for p in get_projects()],
        "audit-security": lambda a: subprocess.run([sys.executable, "tools/security_auditor.py", "."]),
        "audit-performance": lambda a: subprocess.run([sys.executable, "tools/weight_reporter.py", "."]),
        "help": lambda a: cmd_help(console)
    }
    if args.command in cmds: cmds[args.command](args)
    else: cmd_help(console)

if __name__ == "__main__": main()
