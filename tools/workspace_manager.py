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
    ws_table.add_row("[bold cyan]build    [/]", "[dim]Execute HEMTT build[/]")
    ws_table.add_row("[bold cyan]release  [/]", "[dim]Build and Upload to Steam Workshop[/]")
    ws_table.add_row("[bold cyan]lint     [/]", "[dim]Full Quality Suite (MD, JSON, Config, SQF)[/]")
    
    audit_table = Table(title="[Assurance & Quality]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold yellow")
    audit_table.add_row("[bold cyan]audit-security   [/]", "[dim]Scan for leaked tokens/keys[/]")
    audit_table.add_row("[bold cyan]audit-performance[/]", "[dim]Scan for texture bottlenecks[/]")
    audit_table.add_row("[bold cyan]audit-assets     [/]", "[dim]Verify PBO integrity and headers[/]")
    audit_table.add_row("[bold cyan]audit-signatures [/]", "[dim]Verify bikey/bisign matches[/]")
    audit_table.add_row("[bold cyan]audit-deps       [/]", "[dim]Analyze transitive dependencies[/]")
    audit_table.add_row("[bold cyan]audit-strings    [/]", "[dim]Validate stringtables[/]")
    audit_table.add_row("[bold cyan]audit-mission    [/]", "[dim]Verify mission file compliance[/]")
    audit_table.add_row("[bold cyan]audit-updates    [/]", "[dim]Check for upstream Workshop updates[/]")
    
    gen_table = Table(title="[Generation & Templates]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold green")
    gen_table.add_row("[bold cyan]generate-catalog [/]", "[dim]Rebuild ASSET_CATALOG.md[/]")
    gen_table.add_row("[bold cyan]generate-manifest[/]", "[dim]Update project unit_manifest.json[/]")
    gen_table.add_row("[bold cyan]generate-preset  [/]", "[dim]Generate Arma 3 Launcher preset[/]")
    gen_table.add_row("[bold cyan]generate-report  [/]", "[dim]Create Diamond Tier status report[/]")
    gen_table.add_row("[bold cyan]generate-vscode  [/]", "[dim]Refresh .vscode tasks and settings[/]")
    gen_table.add_row("[bold cyan]generate-changelog[/]", "[dim]Build changelog from git history[/]")
    
    intel_table = Table(title="[Intelligence & Maintenance]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold magenta")
    intel_table.add_row("[bold cyan]gh-runs         [/]", "[dim]Monitor GitHub Actions[/]")
    intel_table.add_row("[bold cyan]harvest-terrain [/]", "[dim]Ingest terrain PNG into COP tiles[/]")
    intel_table.add_row("[bold cyan]apply-updates   [/]", "[dim]Run Import Wizard for new assets[/]")
    intel_table.add_row("[bold cyan]fix-syntax      [/]", "[dim]Automated SQF/Config formatting[/]")
    intel_table.add_row("[bold cyan]check-env       [/]", "[dim]Verify local dev environment[/]")
    intel_table.add_row("[bold cyan]self-update     [/]", "[dim]Update UKSFTA-Tools repository[/]")
    
    console.print(ws_table); console.print(audit_table); console.print(gen_table); console.print(intel_table)

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
    for cmd in ["status", "update", "gh-runs", "generate-catalog", "build", "release", "audit-security", "audit-performance", "audit-assets", "audit-signatures", "audit-keys", "audit-deps", "audit-strings", "audit-mission", "audit-updates", "apply-updates", "generate-manifest", "generate-preset", "generate-report", "generate-vscode", "generate-changelog", "fix-syntax", "check-env", "self-update", "help"]:
        subparsers.add_parser(cmd)
    
    p_lint = subparsers.add_parser("lint")
    p_lint.add_argument("--fix", action="store_true")
    
    p_harvest = subparsers.add_parser("harvest-terrain")
    p_harvest.add_argument("image"); p_harvest.add_argument("name")
    
    args = parser.parse_args(); console = Console(force_terminal=True)
    cmds = {
        "gh-runs": cmd_gh_runs,
        "lint": cmd_lint,
        "update": lambda a: [
            (subprocess.run(["git", "submodule", "update", "--init", "--recursive", "--remote", "--force", ".uksf_tools"], cwd=p),
             subprocess.run([sys.executable, str(p/".uksf_tools/setup.py")], cwd=p))
            for p in get_projects()
        ],
        "status": lambda a: [print(f"Project: {p.name}") for p in get_projects()],
        "build": lambda a: [subprocess.run(["bash", "build.sh", "build"], cwd=p) for p in get_projects()],
        "release": lambda a: [subprocess.run([sys.executable, "tools/release.py"], cwd=p) for p in get_projects()],
        "generate-catalog": lambda a: subprocess.run([sys.executable, "tools/catalog_generator.py", "."]),
        "generate-manifest": lambda a: subprocess.run([sys.executable, "tools/manifest_generator.py", "."]),
        "generate-preset": lambda a: subprocess.run([sys.executable, "tools/preset_generator.py", "."]),
        "generate-report": lambda a: subprocess.run([sys.executable, "tools/report_generator.py", "."]),
        "generate-vscode": lambda a: subprocess.run([sys.executable, "tools/vscode_task_generator.py", "."]),
        "generate-changelog": lambda a: subprocess.run([sys.executable, "tools/changelog_generator.py", "."]),
        "audit-security": lambda a: subprocess.run([sys.executable, "tools/security_auditor.py", "."]),
        "audit-performance": lambda a: subprocess.run([sys.executable, "tools/weight_reporter.py", "."]),
        "audit-assets": lambda a: subprocess.run([sys.executable, "tools/asset_auditor.py", "."]),
        "audit-signatures": lambda a: subprocess.run([sys.executable, "tools/key_auditor.py", "."]),
        "audit-keys": lambda a: subprocess.run([sys.executable, "tools/key_auditor.py", "."]),
        "audit-deps": lambda a: subprocess.run([sys.executable, "tools/dependency_graph.py", "."]),
        "audit-strings": lambda a: subprocess.run([sys.executable, "tools/string_auditor.py", "."]),
        "audit-mission": lambda a: subprocess.run([sys.executable, "tools/mission_auditor.py", "."]),
        "audit-updates": lambda a: subprocess.run([sys.executable, "tools/workshop_inspector.py", "."]),
        "apply-updates": lambda a: subprocess.run([sys.executable, "tools/import_wizard.py", "."]),
        "fix-syntax": lambda a: subprocess.run([sys.executable, "tools/syntax_fixer.py", "."]),
        "check-env": lambda a: subprocess.run([sys.executable, "tools/env_checker.py", "."]),
        "self-update": lambda a: subprocess.run(["git", "pull", "origin", "main"]),
        "harvest-terrain": lambda a: subprocess.run([sys.executable, "tools/terrain_harvester.py", a.image, a.name]),
        "help": lambda a: cmd_help(console)
    }
    if args.command in cmds: cmds[args.command](args)
    else: cmd_help(console)

if __name__ == "__main__": main()
