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
import html
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
    from rich.columns import Columns
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
    """Checks if a directory is a valid UKSFTA project."""
    return (path / ".hemtt" / "project.toml").exists() or (path / "mod_sources.txt").exists()

def get_projects():
    """
    Returns a list of projects based on current working directory context.
    """
    cwd = Path.cwd()
    current = cwd
    while current != current.parent:
        if is_project(current) and current.name.startswith("UKSFTA-"):
            return [current]
        current = current.parent

    parent_dir = Path(__file__).parent.parent.parent.resolve()
    projects = []
    if parent_dir.exists():
        for d in parent_dir.iterdir():
            if d.is_dir() and d.name.startswith("UKSFTA-") and is_project(d):
                projects.append(d)
    return sorted(projects)

def get_live_timestamp(mid):
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={mid}"
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=5) as response:
            page = response.read().decode('utf-8')
            ts_match = re.search(r'data-timestamp="(\d+)"', page)
            return ts_match.group(1) if ts_match else "0"
    except: return "0"

def get_dir_size(path):
    total = 0
    with os.scandir(path) as it:
        for entry in it:
            if entry.is_file(): total += entry.stat().st_size
            elif entry.is_dir(): total += get_dir_size(entry.path)
    return total

def format_bytes(size):
    for unit in ['B','KB','MB','GB']:
        if size < 1024: return f"{size:.1f} {unit}"
        size /= 1024
    return f"{size:.1f} TB"

def print_banner(console):
    version = "Unknown"
    v_path = Path(__file__).parent.parent / "VERSION"
    if v_path.exists(): version = v_path.read_text().strip()
    banner = Text.assemble(("\n [!] ", "bold blue"), ("UKSF TASKFORCE ALPHA ", "bold white"), ("| ", "dim"), ("PLATINUM DEVOPS SUITE ", "bold cyan"), (f"v{version}", "bold yellow"), ("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n", "dim blue"))
    console.print(banner)

def cmd_help(console):
    print_banner(console)
    ws_table = Table(title="[Workspace Operations]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold cyan")
    ws_table.add_row("[bold cyan]status   [/]", "[dim]Show git status summary for current context[/]")
    ws_table.add_row("[bold cyan]sync     [/]", "[dim]Pull latest Workshop updates and synchronize mods[/]")
    ws_table.add_row("[bold cyan]update   [/]", "[dim]Propagate latest UKSFTA-Tools to projects[/]")
    ws_table.add_row("[bold cyan]self-update[/]", "[dim]Pull latest DevOps improvements from Tools repo[/]")
    ws_table.add_row("[bold cyan]cache    [/]", "[dim]Show disk space usage of build artifacts[/]")
    ws_table.add_row("[bold cyan]clean    [/]", "[dim]Wipe all .hemttout build artifacts[/]")
    ws_table.add_row("[bold cyan]check-env[/]", "[dim]Verify local development tools and dependencies[/]")
    intel_table = Table(title="[Unit Intelligence]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold magenta")
    intel_table.add_row("[bold cyan]dashboard       [/]", "[dim]Visual overview of projects, components, and versions[/]")
    intel_table.add_row("[bold cyan]workshop-info   [/]", "[dim]Query live versions and timestamps from Steam Workshop[/]")
    intel_table.add_row("[bold cyan]modlist-size    [/]", "[dim]Calculate total data size of any Arma 3 modlist[/]")
    intel_table.add_row("[bold cyan]modlist-classify[/]", "[dim]Audit an entire modlist for side requirements[/]")
    intel_table.add_row("[bold cyan]modlist-audit   [/]", "[dim]Compare a modlist against one or more sources[/]")
    intel_table.add_row("[bold cyan]classify-mod    [/]", "[dim]Deep audit of a single mod's side requirement[/]")
    intel_table.add_row("[bold cyan]audit-updates   [/]", "[dim]Check live Workshop for pending mod updates[/]")
    intel_table.add_row("[bold cyan]apply-updates   [/]", "[dim]Automatically update and sync all out-of-date mods[/]")
    intel_table.add_row("[bold cyan]gh-runs         [/]", "[dim]Real-time monitoring of GitHub Actions runners[/]")
    intel_table.add_row("[bold cyan]trend-analyze   [/]", "[dim]Track and report on unit health score trends[/]")
    intel_table.add_row("[bold cyan]plan            [/]", "[dim]The Architect: Reasoning agent for strategic analysis[/]")
    audit_table = Table(title="[Assurance & Quality]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold yellow")
    audit_table.add_row("[bold cyan]audit            [/]", "[dim]Master Audit: Run all health and security checks[/]")
    audit_table.add_row("[bold cyan]lint             [/]", "[dim]Full Quality Suite (Markdown, JSON, Config, SQF)[/]")
    audit_table.add_row("[bold cyan]test             [/]", "[dim]Run full suite (pytest, hemtt check, sqflint)[/]")
    audit_table.add_row("[bold cyan]audit-performance[/]", "[dim]Scan assets for texture and model optimization issues[/]")
    audit_table.add_row("[bold cyan]audit-signatures [/]", "[dim]Verify PBO signing state and unit key matches[/]")
    audit_table.add_row("[bold cyan]audit-keys       [/]", "[dim]Verify presence of official unit public keys[/]")
    audit_table.add_row("[bold cyan]audit-deps       [/]", "[dim]Scan requiredAddons for missing dependencies[/]")
    audit_table.add_row("[bold cyan]audit-assets     [/]", "[dim]Detect orphaned/unused binary files (PAA, P3D)[/]")
    audit_table.add_row("[bold cyan]audit-strings    [/]", "[dim]Validate stringtable keys vs SQF usage[/]")
    audit_table.add_row("[bold cyan]audit-security   [/]", "[dim]Scan for leaked tokens, webhooks, or private keys[/]")
    audit_table.add_row("[bold cyan]audit-mission    [/]", "[dim]Verify a Mission PBO against workspace and externals[/]")
    audit_table.add_row("[bold cyan]audit-preset     [/]", "[dim]Compliance check for external Workshop modlists[/]")
    prod_table = Table(title="[Production & Utilities]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold green")
    prod_table.add_row("[bold cyan]build            [/]", "[dim]Execute HEMTT build on projects in context[/]")
    prod_table.add_row("[bold cyan]release          [/]", "[dim]Generate signed/packaged release ZIPs[/]")
    prod_table.add_row("[bold cyan]publish          [/]", "[dim]Upload projects to Steam Workshop[/]")
    prod_table.add_row("[bold cyan]mission-setup    [/]", "[dim]Unit-standardize a mission folder (new or existing)[/]")
    prod_table.add_row("[bold cyan]generate-preset  [/]", "[dim]Create master HTML preset of all unit dependencies[/]")
    prod_table.add_row("[bold cyan]generate-report  [/]", "[dim]Create a Markdown health report for context[/]")
    prod_table.add_row("[bold cyan]generate-manifest[/]", "[dim]Create unit-wide manifest of all mods and PBOs[/]")
    prod_table.add_row("[bold cyan]generate-changelog[/]", "[dim]Create detailed asset changelogs for projects[/]")
    prod_table.add_row("[bold cyan]generate-catalog [/]", "[dim]Generate a visual armory catalog of assets[/]")
    prod_table.add_row("[bold cyan]generate-vscode  [/]", "[dim]Setup VS Code Tasks for one-click development[/]")
    prod_table.add_row("[bold cyan]fix-syntax       [/]", "[dim]Standardize indentation and formatting in repos[/]")
    prod_table.add_row("[bold cyan]import-wizard    [/]", "[dim]One-click automated ingestion of external assets[/]")
    prod_table.add_row("[bold cyan]unit-wide-sync   [/]", "[dim]Automated bulk normalization of all unit repositories[/]")
    prod_table.add_row("[bold cyan]optimize-assets  [/]", "[dim]Active texture downscaling and optimization[/]")
    prod_table.add_row("[bold cyan]watch            [/]", "[dim]The Sentinel: Autonomous real-time asset watchdog[/]")
    prod_table.add_row("[bold cyan]ace-arsenal      [/]", "[dim]Automated grouping for ACE Extended Arsenal[/]")
    console.print(ws_table); console.print(intel_table); console.print(audit_table); console.print(prod_table)
    console.print("\n[bold]Context Awareness:[/bold] Running in a project directory will only affect that project.\n")

def cmd_dashboard(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "platinum_score.py"
    subprocess.run([sys.executable, str(auditor)])

def cmd_gh_runs(args):
    projects = get_projects(); workflow_names = set(); all_stats = []
    for p in projects:
        try:
            res = subprocess.run(["gh", "run", "list", "--limit", "15", "--json", "workflowName"], cwd=p, capture_output=True, text=True)
            if res.returncode == 0:
                for r in json.loads(res.stdout): workflow_names.add(r['workflowName'])
        except: pass
    sorted_workflows = sorted(list(workflow_names))
    for p in projects:
        stats = {"project": p.name, "workflows": {}, "latest_age": "-"}
        try:
            res = subprocess.run(["gh", "run", "list", "--limit", "20", "--json", "workflowName,conclusion,status,createdAt"], cwd=p, capture_output=True, text=True)
            if res.returncode == 0:
                for run in json.loads(res.stdout):
                    wf = run['workflowName']
                    if wf not in stats["workflows"]:
                        stats["workflows"][wf] = {"status": run['status'], "conclusion": run['conclusion']}
                        if stats["latest_age"] == "-":
                            created = datetime.fromisoformat(run['createdAt'].replace('Z', '+00:00'))
                            diff = datetime.now(created.tzinfo) - created
                            stats["latest_age"] = f"{diff.days}d" if diff.days > 0 else f"{diff.seconds // 3600}h"
        except: pass
        all_stats.append(stats)
    if args.json: print(json.dumps(all_stats, indent=2)); return
    console = Console(force_terminal=True); print_banner(console)
    if not sorted_workflows: console.print("[yellow]! No runs found.[/yellow]"); return
    display_names = {wf: os.path.basename(wf).replace(".yml", "").capitalize() for wf in sorted_workflows}
    table = Table(title="Pipeline Matrix", box=box.ROUNDED, border_style="blue")
    table.add_column("Project", style="cyan"); [table.add_column(display_names[wf], justify="center") for wf in sorted_workflows]; table.add_column("Age", justify="right")
    for s in all_stats:
        row_icons = []
        for wf in sorted_workflows:
            w_stat = s["workflows"].get(wf, {"status": "none", "conclusion": "none"})
            if w_stat["status"] == "none": icon = "-"
            elif w_stat["status"] != "completed": icon = "..."
            elif w_stat["conclusion"] == "success": icon = "[bold green]PASS[/]"
            else: icon = "[bold red]FAIL[/]"
            row_icons.append(icon)
        table.add_row(s["project"], *row_icons, s["latest_age"])
    console.print(table)

def cmd_audit_updates(args):
    projects = get_projects(); mod_registry = {}
    for p in projects:
        lock_path = p / "mods.lock"
        if lock_path.exists():
            with open(lock_path, 'r') as f:
                for mid, info in json.load(f).get("mods", {}).items():
                    if mid not in mod_registry: mod_registry[mid] = {"name": info['name'], "locked": info.get('updated', '0'), "project": p.name}
    if not mod_registry: return
    def check_mod(mid): return mid, get_live_timestamp(mid)
    with ThreadPoolExecutor(max_workers=10) as executor: live_results = dict(executor.map(check_mod, mod_registry.keys()))
    results = []
    for mid, data in mod_registry.items():
        live_ts = live_results.get(mid, "0")
        results.append({"mid": mid, "name": data["name"], "project": data["project"], "locked": data["locked"], "live": live_ts, "up_to_date": data["locked"] == live_ts})
    if args.json: print(json.dumps(results, indent=2)); return
    console = Console(force_terminal=True); print_banner(console)
    table = Table(title="Update Audit", box=box.ROUNDED, border_style="yellow")
    table.add_column("Project", style="cyan"); table.add_column("Mod", style="magenta"); table.add_column("Locked"); table.add_column("Live"); table.add_column("Status")
    for r in results:
        status = "[bold green]LATEST[/bold green]" if r["up_to_date"] else "[bold red]UPDATE[/bold red]"
        table.add_row(r["project"], r["name"], r["locked"], r["live"], status)
    console.print(table)

def cmd_status(args):
    projects = get_projects(); all_status = []
    for p in projects:
        res = subprocess.run(["git", "status", "-s"], cwd=p, capture_output=True, text=True)
        all_status.append({"project": p.name, "dirty": len(res.stdout.strip()) > 0, "summary": res.stdout.strip()})
    if args.json: print(json.dumps(all_status, indent=2)); return
    console = Console(force_terminal=True); print_banner(console)
    for s in all_status:
        p_obj = next(p for p in projects if p.name == s["project"])
        console.print(Panel(f"[dim]Root: {p_obj}[/dim]\n{s['summary'] if s['dirty'] else '[green]Clean[/green]'}", title=f"Project: {s['project']}", border_style="cyan"))

def cmd_apply_updates(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    for p in projects:
        lock_path = p / "mods.lock"
        if not lock_path.exists(): continue
        with open(lock_path, 'r') as f: data = json.load(f).get("mods", {})
        updates_found = False
        for mid, info in data.items():
            live_ts = get_live_timestamp(mid)
            if live_ts != "0" and info.get("updated") != live_ts:
                console.print(f"   [bold green]UPDATE[/bold green]: {info['name']} in {p.name}"); info["updated"] = live_ts; updates_found = True
        if updates_found:
            with open(lock_path, 'w') as f: json.dump({"mods": data}, f, indent=2)
            subprocess.run([sys.executable, "tools/manage_mods.py", "sync"], cwd=p); subprocess.run(["git", "add", "mods.lock"], cwd=p); subprocess.run(["git", "commit", "-S", "-m", "chore: automated workshop updates"], cwd=p); subprocess.run(["git", "push", "origin", "main"], cwd=p)

def cmd_self_update(args):
    console = Console(force_terminal=True); print_banner(console); subprocess.run(["git", "pull", "origin", "main"], cwd=Path(__file__).parent.parent)

def cmd_audit_full(args):
    console = Console(force_terminal=True); print_banner(console); console.print(Panel("[bold yellow]🚀 STARTING CONTEXT AUDIT[/bold yellow]", border_style="yellow"))
    cmd_audit_updates(args); cmd_audit_deps(args); cmd_audit_assets(args); cmd_audit_strings(args); cmd_audit_security(args); cmd_audit_signatures(args); cmd_audit_keys(args)

def cmd_lint(args):
    console = Console(force_terminal=True); print_banner(console)
    console.print(Panel("[bold cyan]🚀 STARTING CONTEXT QUALITY LINT[/bold cyan]", border_style="cyan"))
    cmd_md = ["npx", "--yes", "markdownlint-cli2", "**/*.md", "--config", ".github/linters/.markdownlint.json"]
    if args.fix: cmd_md.append("--fix")
    subprocess.run(cmd_md)
    cmd_biome = ["npx", "--yes", "@biomejs/biome", "ci", "."]
    if args.fix: cmd_biome = ["npx", "--yes", "@biomejs/biome", "check", "--write", "."]
    subprocess.run(cmd_biome)
    for p in get_projects():
        console.print(f"\n[bold]3. Project Audit: {p.name}[/bold]")
        subprocess.run([sys.executable, "tools/config_style_checker.py", str(p)])
        subprocess.run([sys.executable, "tools/sqf_validator.py", str(p)])
        subprocess.run([sys.executable, "tools/stringtable_validator.py", str(p)])

def cmd_sync(args):
    console = Console(force_terminal=True); print_banner(console)
    for p in get_projects(): 
        cmd = [sys.executable, "tools/manage_mods.py", "sync"]
        if args.offline: cmd.append("--offline")
        subprocess.run(cmd, cwd=p)

def cmd_build(args):
    for p in get_projects(): subprocess.run(["bash", "build.sh", "build"], cwd=p)

def cmd_release(args):
    central_dir = Path(__file__).parent.parent / "all_releases"; central_dir.mkdir(exist_ok=True)
    for p in get_projects(): 
        subprocess.run(["bash", "build.sh", "release"], cwd=p); proj_releases = p / "releases"
        if proj_releases.exists():
            for zf in proj_releases.glob("*.zip"): shutil.move(str(zf), str(central_dir / zf.name))
            shutil.rmtree(str(proj_releases), ignore_errors=True)

def cmd_publish(args):
    projects = get_projects(); publishable = []
    for p in projects:
        cp = p / ".hemtt" / "project.toml"
        if cp.exists():
            with open(cp, 'r') as f:
                wm = re.search(r'workshop_id = "(.*)"', f.read())
                if wm and wm.group(1).isdigit(): publishable.append((p, wm.group(1)))
    for p, ws_id in publishable:
        cmd = [sys.executable, "tools/release.py", "-n", "-y"]
        if args.dry_run: cmd.append("--dry-run")
        subprocess.run(cmd, cwd=p)

def cmd_update(args):
    setup = Path(__file__).parent.parent / "setup.py"
    for p in get_projects(): subprocess.run([sys.executable, str(setup.resolve())], cwd=p)

def cmd_audit_deps(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "dependency_graph.py"
    subprocess.run([sys.executable, str(auditor)])

def cmd_audit_assets(args):
    auditor = Path(__file__).parent / "asset_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_audit_strings(args):
    auditor = Path(__file__).parent / "string_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_audit_security(args):
    auditor = Path(__file__).parent / "security_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_audit_signatures(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    for p in projects:
        build_addons = p / ".hemttout" / "build" / "addons"
        if not build_addons.exists(): continue
        for pbo in build_addons.glob("*.pbo"):
            signed = (build_addons / f"{pbo.name}.uksfta.bisign").exists()
            print(f"  {p.name:<20} | {pbo.name:<30} | {'✅' if signed else '❌'}")

def cmd_audit_keys(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "key_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_clean(args):
    for p in get_projects():
        target = p / ".hemttout"
        if target.exists(): shutil.rmtree(target); print(f"  ✅ Cleaned: {p.name}")

def cmd_cache(args):
    total = 0
    for p in get_projects():
        target = p / ".hemttout"
        if target.exists(): sz = get_dir_size(target); total += sz; print(f"  {p.name:<20} : {format_bytes(sz)}")
    print(f"\n[bold]Total Cache in Context:[/] {format_bytes(total)}")

def main():
    parser = argparse.ArgumentParser(description="UKSF Taskforce Alpha Manager", add_help=False)
    parser.add_argument("--json", action="store_true", help="Output results in machine-readable JSON format")
    subparsers = parser.add_subparsers(dest="command")
    
    # Generic commands
    for cmd in ["dashboard", "status", "build", "release", "test", "clean", "cache", "validate", "audit", "audit-updates", "apply-updates", "audit-deps", "audit-assets", "audit-strings", "audit-security", "audit-signatures", "audit-performance", "audit-keys", "generate-docs", "generate-manifest", "generate-preset", "generate-report", "generate-vscode", "generate-changelog", "generate-catalog", "setup-git-hooks", "check-env", "fix-syntax", "clean-strings", "update", "self-update", "workshop-tags", "gh-runs", "workshop-info", "help"]:
        subparsers.add_parser(cmd, help=f"Run {cmd} utility")
    
    p_lint = subparsers.add_parser("lint", help="Full Quality Lint"); p_lint.add_argument("--fix", action="store_true")
    p_ms = subparsers.add_parser("mission-setup", help="Standardize a mission folder"); p_ms.add_argument("path")
    p_sync = subparsers.add_parser("sync", help="Synchronize mods"); p_sync.add_argument("--offline", action="store_true")
    p_pub = subparsers.add_parser("publish", help="Upload to Steam"); p_pub.add_argument("--dry-run", action="store_true")
    p_audit_preset = subparsers.add_parser("audit-preset", help="Audit all mods in a Launcher preset"); p_audit_preset.add_argument("file"); p_audit_preset.add_argument("--deep", action="store_true")
    p_wizard = subparsers.add_parser("import-wizard", help="Automated asset porting wizard"); p_wizard.add_argument("source"); p_wizard.add_argument("name"); p_wizard.add_argument("prefix")
    p_unit_sync = subparsers.add_parser("unit-wide-sync", help="Bulk normalize all unit repositories"); p_unit_sync.add_argument("old_tag")
    p_opt = subparsers.add_parser("optimize-assets", help="Active asset optimization"); p_opt.add_argument("path"); p_opt.add_argument("--apply", action="store_true")
    p_trend = subparsers.add_parser("trend-analyze", help="Track unit health trends"); p_trend.add_argument("--report", action="store_true")
    p_plan = subparsers.add_parser("plan", help="The Architect: Strategic reasoning agent")
    p_watch = subparsers.add_parser("watch", help="The Sentinel: Autonomous real-time watchdog"); p_watch.add_argument("path", nargs="?", default=".")
    p_arsenal = subparsers.add_parser("ace-arsenal", help="Automated ACE Extended Arsenal grouping"); p_arsenal.add_argument("config")
    
    args = parser.parse_args(); console = Console(force_terminal=True)
    cmds = {
        "dashboard": cmd_dashboard, "status": cmd_status, "sync": cmd_sync, "build": cmd_build, "release": cmd_release,
        "test": lambda a: subprocess.run(["pytest"]), "clean": cmd_clean, "cache": cmd_cache,
        "audit": cmd_audit_full, "audit-updates": cmd_audit_updates, "apply-updates": cmd_apply_updates, "audit-deps": cmd_audit_deps,
        "audit-assets": cmd_audit_assets, "audit-strings": cmd_audit_strings, "audit-security": cmd_audit_security, "audit-signatures": cmd_audit_signatures,
        "audit-performance": lambda a: subprocess.run([sys.executable, "tools/weight_reporter.py", "."]),
        "audit-preset": lambda a: subprocess.run([sys.executable, "tools/modlist_auditor.py", a.file] + (["--deep"] if a.deep else [])),
        "generate-catalog": lambda a: subprocess.run([sys.executable, "tools/catalog_generator.py"]),
        "optimize-assets": lambda a: subprocess.run([sys.executable, "tools/asset_optimizer.py", a.path] + (["--apply"] if a.apply else [])),
        "trend-analyze": lambda a: subprocess.run([sys.executable, "tools/trend_analyzer.py"] + (["report"] if a.report else [])),
        "plan": lambda a: subprocess.run([sys.executable, "tools/agent_architect.py"]),
        "watch": lambda a: subprocess.run([sys.executable, "tools/agent_sentinel.py", a.path]),
        "ace-arsenal": lambda a: subprocess.run([sys.executable, "tools/ace_arsenal_helper.py", a.config]),
        "import-wizard": lambda a: subprocess.run([sys.executable, "tools/import_wizard.py", a.source, a.name, a.prefix]),
        "unit-wide-sync": lambda a: [subprocess.run([sys.executable, "tools/path_refactor.py", str(p), a.old_tag]) for p in get_projects()],
        "lint": cmd_lint, "help": lambda a: cmd_help(console)
    }
    if args.command in cmds: cmds[args.command](args)
    else: cmd_help(console)

if __name__ == "__main__": main()
