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

def get_projects():
    parent_dir = Path(__file__).parent.parent.parent
    projects = []
    for d in parent_dir.iterdir():
        if d.is_dir() and d.name.startswith("UKSFTA-") and (d / ".hemtt" / "project.toml").exists():
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

def print_banner(console):
    version = "Unknown"
    v_path = Path(__file__).parent.parent / "VERSION"
    if v_path.exists(): version = v_path.read_text().strip()
    
    banner = Text.assemble(
        ("\n [!] ", "bold blue"),
        ("UKSF TASKFORCE ALPHA ", "bold white"),
        ("| ", "dim"),
        ("PLATINUM DEVOPS SUITE ", "bold cyan"),
        (f"v{version}", "bold yellow"),
        ("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n", "dim blue")
    )
    console.print(banner)

def cmd_help(console):
    print_banner(console)
    ws_table = Table(title="[Workspace Operations]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold cyan")
    ws_table.add_row("[bold cyan]status   [/]", "[dim]Show git status summary for every repository[/]")
    ws_table.add_row("[bold cyan]sync     [/]", "[dim]Pull latest Workshop updates and synchronize mods[/]")
    ws_table.add_row("[bold cyan]update   [/]", "[dim]Propagate latest UKSFTA-Tools to all projects[/]")
    ws_table.add_row("[bold cyan]self-update[/]", "[dim]Pull latest DevOps improvements from Tools repo[/]")
    ws_table.add_row("[bold cyan]cache    [/]", "[dim]Show disk space usage of build artifacts[/]")
    ws_table.add_row("[bold cyan]clean    [/]", "[dim]Wipe all .hemttout build artifacts[/]")
    ws_table.add_row("[bold cyan]check-env[/]", "[dim]Verify local development tools and dependencies[/]")
    intel_table = Table(title="[Unit Intelligence]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold magenta")
    intel_table.add_row("[bold cyan]dashboard       [/]", "[dim]Visual overview of all projects, components, and versions[/]")
    intel_table.add_row("[bold cyan]workshop-info   [/]", "[dim]Query live versions and timestamps from Steam Workshop[/]")
    intel_table.add_row("[bold cyan]modlist-size    [/]", "[dim]Calculate total data size of any Arma 3 modlist[/]")
    intel_table.add_row("[bold cyan]modlist-classify[/]", "[dim]Audit an entire modlist for side requirements[/]")
    intel_table.add_row("[bold cyan]modlist-audit   [/]", "[dim]Compare a modlist against one or more sources[/]")
    intel_table.add_row("[bold cyan]classify-mod    [/]", "[dim]Deep audit of a single mod's side requirement[/]")
    intel_table.add_row("[bold cyan]audit-updates   [/]", "[dim]Check live Workshop for pending mod updates[/]")
    intel_table.add_row("[bold cyan]apply-updates   [/]", "[dim]Automatically update and sync all out-of-date mods[/]")
    intel_table.add_row("[bold cyan]gh-runs         [/]", "[dim]Real-time monitoring of GitHub Actions runners[/]")
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
    prod_table = Table(title="[Production & Utilities]", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold green")
    prod_table.add_row("[bold cyan]build            [/]", "[dim]Execute HEMTT build on all projects[/]")
    prod_table.add_row("[bold cyan]release          [/]", "[dim]Generate signed/packaged release ZIPs[/]")
    prod_table.add_row("[bold cyan]publish          [/]", "[dim]Upload projects to Steam Workshop[/]")
    prod_table.add_row("[bold cyan]mission-setup    [/]", "[dim]Unit-standardize a mission folder (new or existing)[/]")
    prod_table.add_row("[bold cyan]generate-preset  [/]", "[dim]Create master HTML preset of all unit dependencies[/]")
    prod_table.add_row("[bold cyan]generate-report  [/]", "[dim]Create a Markdown health report for the entire unit[/]")
    prod_table.add_row("[bold cyan]generate-manifest[/]", "[dim]Create unit-wide manifest of all mods and PBOs[/]")
    prod_table.add_row("[bold cyan]generate-vscode  [/]", "[dim]Setup VS Code Tasks for one-click development[/]")
    prod_table.add_row("[bold cyan]setup-git-hooks  [/]", "[dim]Install local pre-commit quality/security guards[/]")
    prod_table.add_row("[bold cyan]fix-syntax       [/]", "[dim]Standardize indentation and formatting in all repos[/]")
    prod_table.add_row("[bold cyan]clean-strings    [/]", "[dim]Purge unused keys from all stringtable.xml files[/]")
    prod_table.add_row("[bold cyan]notify           [/]", "[dim]Send a manual development update to Discord[/]")
    prod_table.add_row("[bold cyan]generate-docs    [/]", "[dim]Auto-generate API Manual from SQF headers[/]")
    prod_table.add_row("[bold cyan]convert          [/]", "[dim]Optimize media for Arma (WAV/PNG -> OGG/PAA)[/]")
    prod_table.add_row("[bold cyan]workshop-tags    [/]", "[dim]List all valid Arma 3 Steam Workshop tags[/]")
    prod_table.add_row("[bold cyan]remote setup     [/]", "[dim]Onboard VPS: remote setup user@host [name][/]")
    prod_table.add_row("[bold cyan]remote           [/]", "[dim]Manage distributed VPS nodes and remote DevOps[/]")
    console.print(ws_table); console.print(intel_table); console.print(audit_table); console.print(prod_table)
    console.print("\n[bold]Tip:[/bold] Run [cyan]./tools/workspace_manager.py <command> --help[/cyan] for detailed options and examples.\n")

def cmd_dashboard(args):
    projects = get_projects(); results = []
    for p in projects:
        version = "0.0.0"; pbos = []; ext_count = 0; sync_state = "OK"
        addons_dir = p / "addons"; sources_path = p / "mod_sources.txt"; lock_path = p / "mods.lock"
        if addons_dir.exists():
            for entry in addons_dir.iterdir():
                if entry.is_dir() and not entry.name.startswith("."): pbos.append(entry.name)
                elif entry.suffix.lower() == ".pbo": pbos.append(entry.stem)
        if sources_path.exists():
            with open(sources_path, 'r') as f:
                for line in f:
                    if re.search(r"(\d{8,})", line) and "[ignore]" not in line.lower() and "ignore=" not in line.lower(): ext_count += 1
        if ext_count > 0 and not lock_path.exists(): sync_state = "PENDING"
        v_path = p / "addons" / "main" / "script_version.hpp"
        if v_path.exists():
            with open(v_path, 'r') as f:
                vc = f.read(); ma = re.search(r'#define MAJOR (.*)', vc); mi = re.search(r'#define MINOR (.*)', vc); pa = re.search(r'#define PATCHLVL (.*)', vc)
                if ma and mi and pa: version = f"{ma.group(1).strip()}.{mi.group(1).strip()}.{pa.group(1).strip()}"
        results.append({"project": p.name, "version": version, "pbos": pbos, "external_count": ext_count, "sync_state": sync_state})
    if args.json: print(json.dumps(results, indent=2)); return
    console = Console(force_terminal=True); print_banner(console)
    table = Table(title=f"Unit Workspace Overview ({len(projects)} Projects)", box=box.ROUNDED, header_style="bold magenta", border_style="blue")
    table.add_column("Project", style="cyan", no_wrap=True); table.add_column("Version", style="bold yellow")
    table.add_column("Components (PBOs)", style="dim"); table.add_column("External Mods", justify="center")
    table.add_column("Sync State", justify="center")
    for r in results:
        pbo_str = ", ".join(r["pbos"][:3]) + (f" (+{len(r['pbos'])-3})" if len(r["pbos"])>3 else "")
        sync_color = "[bold green]OK[/bold green]" if r["sync_state"] == "OK" else "[bold red]PENDING[/bold red]"
        table.add_row(r["project"], r["version"], pbo_str if r["pbos"] else "-", str(r["external_count"]) if r["external_count"] else "-", sync_color)
    console.print(table)

def cmd_gh_runs(args):
    projects = get_projects(); workflow_names = set(); all_stats = []
    for p in projects:
        try:
            res = subprocess.run(["gh", "run", list, "--limit", "15", "--json", "workflowName"], cwd=p, capture_output=True, text=True)
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
    console.print(table); console.print("[dim]Key: PASS | FAIL | ... Running | - No Data[/dim]")

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
    console = Console(force_terminal=True); print_banner(console)
    console.print("[bold yellow]🚀 UPDATING PLATINUM DEVOPS TOOLS[/bold yellow]")
    root = Path(__file__).parent.parent
    subprocess.run(["git", "pull", "origin", "main"], cwd=root)
    console.print("[bold green]✅ Tools updated to latest version.[/bold green]")

def cmd_audit_signatures(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    table = Table(title="Signature Audit", box=box.ROUNDED, border_style="magenta")
    table.add_column("Project", style="cyan"); table.add_column("PBO", style="dim"); table.add_column("Signed", justify="center")
    for p in projects:
        build_addons = p / ".hemttout" / "build" / "addons"
        if not build_addons.exists(): continue
        for pbo in build_addons.glob("*.pbo"):
            signed = (build_addons / f"{pbo.name}.uksfta.bisign").exists() or (build_addons / f"{pbo.name}.UKSFTA.bisign").exists()
            table.add_row(p.name, pbo.name, "[bold green]SIGNED[/bold green]" if signed else "[bold red]UNSIGNED[/bold red]")
    console.print(table)

def cmd_audit_keys(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "key_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_notify(args):
    notifier = Path(__file__).parent / "notify_discord.py"; cmd = [sys.executable, str(notifier), "--message", args.message, "--type", args.type]
    if args.title: cmd.extend(["--title", args.title])
    if args.dry_run: cmd.append("--dry-run")
    subprocess.run(cmd)

def cmd_audit_full(args):
    console = Console(force_terminal=True); print_banner(console); console.print(Panel("[bold yellow]🚀 STARTING GLOBAL UNIT AUDIT[/bold yellow]", border_style="yellow"))
    cmd_audit_updates(args); cmd_audit_deps(args); cmd_audit_assets(args); cmd_audit_strings(args); cmd_audit_security(args); cmd_audit_signatures(args); cmd_audit_keys(args)

def cmd_lint(args):
    console = Console(force_terminal=True); print_banner(console)
    console.print(Panel("[bold cyan]🚀 STARTING GLOBAL QUALITY LINT[/bold cyan]", border_style="cyan"))
    
    # 1. Markdown Linting
    console.print("\n[bold]1. Markdown Audit:[/bold]")
    cmd_md = ["npx", "--yes", "markdownlint-cli2", "**/*.md", "--config", ".github/linters/.markdownlint.json"]
    if args.fix: cmd_md.append("--fix")
    subprocess.run(cmd_md)

    # 2. JSON/Metadata (Biome)
    console.print("\n[bold]2. JSON & Metadata (Biome):[/bold]")
    cmd_biome = ["npx", "--yes", "@biomejs/biome", "ci", "."]
    if args.fix: cmd_biome = ["npx", "--yes", "@biomejs/biome", "check", "--write", "."]
    subprocess.run(cmd_biome)

    # 3. Ansible Audit
    console.print("\n[bold]3. Ansible Infrastructure Audit:[/bold]")
    if shutil.which("ansible-lint"):
        subprocess.run(["ansible-lint", "remote/"])
    else:
        console.print("[yellow]! Skipping: ansible-lint not found.[/yellow]")

    # 4. Project Checkers
    projects = get_projects()
    for p in projects:
        console.print(f"\n[bold]4. Project Audit: {p.name}[/bold]")
        # Config
        subprocess.run([sys.executable, str(Path(__file__).parent / "config_style_checker.py"), str(p)])
        # SQF
        subprocess.run([sys.executable, str(Path(__file__).parent / "sqf_validator.py"), str(p)])
        # Strings
        subprocess.run([sys.executable, str(Path(__file__).parent / "stringtable_validator.py"), str(p)])

def cmd_mission_setup(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "mission_scaffolder.py"
    cmd = [sys.executable, str(auditor), args.path]
    if args.framework: cmd.append("--framework")
    subprocess.run(cmd)

def cmd_generate_preset(args):
    auditor = Path(__file__).parent / "preset_generator.py"; subprocess.run([sys.executable, str(auditor)])

def cmd_generate_report(args):
    auditor = Path(__file__).parent / "report_generator.py"; subprocess.run([sys.executable, str(auditor)])

def cmd_generate_changelog(args):
    tool = Path(__file__).parent / "changelog_generator.py"
    for p in get_projects(): subprocess.run([sys.executable, str(tool), str(p)])

def cmd_generate_vscode(args):
    tool = Path(__file__).parent / "vscode_task_generator.py"
    for p in get_projects(): subprocess.run([sys.executable, str(tool), str(p)])

def cmd_setup_git_hooks(args):
    tool = Path(__file__).parent / "git_hook_installer.py"
    for p in get_projects(): subprocess.run([sys.executable, str(tool), str(p)])

def cmd_check_env(args):
    tool = Path(__file__).parent / "env_checker.py"; subprocess.run([sys.executable, str(tool)])

def cmd_fix_syntax(args):
    fixer = Path(__file__).parent / "syntax_fixer.py"
    cmd = [sys.executable, str(fixer), args.target]
    if args.dry_run: cmd.append("--dry-run")
    subprocess.run(cmd)

def cmd_clean_strings(args):
    cleaner = Path(__file__).parent / "string_cleaner.py"
    for p in get_projects(): subprocess.run([sys.executable, str(cleaner), str(p)])

def cmd_audit_performance(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "performance_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_classify_mod(args):
    tool = Path(__file__).parent / "mod_classifier.py"; subprocess.run([sys.executable, str(tool), args.id])

def cmd_modlist_classify(args):
    tool = Path(__file__).parent / "modlist_classifier.py"; subprocess.run([sys.executable, str(tool), args.file])

def cmd_modlist_audit(args):
    tool = Path(__file__).parent / "modlist_auditor.py"
    cmd = [sys.executable, str(tool), args.reference] + args.targets
    if args.deep: cmd.append("--deep")
    subprocess.run(cmd)

def cmd_audit_deps(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects(); defined_patches = set(); dependencies = {}
    for p in projects:
        for config in p.glob("addons/*/config.cpp"):
            with open(config, 'r', errors='ignore') as f:
                content = f.read()
                for m in re.finditer(r'class\s+CfgPatches\s*\{[^}]*class\s+([a-zA-Z0-9_]+)', content, re.MULTILINE | re.DOTALL): defined_patches.add(m.group(1))
                rm = re.search(r'requiredAddons\[\]\s*=\s*\{([^}]*)\}', content, re.MULTILINE | re.DOTALL)
                if rm: dependencies[config] = [r.strip().replace('"', '').replace("'", "") for r in rm.group(1).split(',') if r.strip()]
    table = Table(title="Dependency Scan", box=box.ROUNDED, border_style="blue")
    table.add_column("Config File", style="dim"); table.add_column("Health", justify="center"); table.add_column("Issues", style="bold red")
    for cfg, reqs in dependencies.items():
        rel = cfg.relative_to(Path(__file__).parent.parent.parent); exts = ["A3_", "cba_", "ace_", "task_force_radio", "acre_", "rhsusf_", "rhs_", "cup_", "uk3cb_", "Peral_", "Arlit_"]
        miss = [r for r in reqs if r not in defined_patches and not any(r.lower().startswith(x.lower()) for x in exts)]
        if miss: table.add_row(str(rel), "❌ [bold red]FAIL[/bold red]", ", ".join(miss))
        else: table.add_row(str(rel), "✅ [bold green]PASS[/bold green]", "[dim]Healthy[/dim]")
    console.print(table)

def cmd_audit_mission(args):
    console = Console(force_terminal=True); print_banner(console); from mission_auditor import audit_mission; defined_patches = set()
    for p in get_projects():
        for config in p.glob("addons/*/config.cpp"):
            with open(config, 'r', errors='ignore') as f:
                for m in re.finditer(r'class\s+CfgPatches\s*\{[^}]*class\s+([a-zA-Z0-9_]+)', f.read(), re.MULTILINE | re.DOTALL): defined_patches.add(m.group(1))
    results = audit_mission(args.pbo, defined_patches)
    if not results: return
    table = Table(title="Mission Analysis", box=box.ROUNDED, border_style="blue")
    for m in results["missing"]: table.add_row("Missing", m, "[bold red]❌[/bold red]")
    for l in results["local"]: table.add_row("UKSFTA", l, "[bold green]✅[/bold green]")
    for e in results["external"]: table.add_row("External", e, "[bold blue]ℹ️[/bold blue]")
    console.print(table)

def cmd_audit_assets(args):
    auditor = Path(__file__).parent / "asset_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_audit_strings(args):
    auditor = Path(__file__).parent / "string_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_audit_security(args):
    auditor = Path(__file__).parent / "security_auditor.py"
    for p in get_projects(): subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_sync(args):
    console = Console(force_terminal=True); print_banner(console)
    for p in get_projects(): 
        cmd = [sys.executable, "tools/manage_mods.py", "sync"]
        if args.offline: cmd.append("--offline")
        if args.dry_run: cmd.append("--dry-run")
        subprocess.run(cmd, cwd=p)
    from manifest_generator import generate_total_manifest; generate_total_manifest(Path(__file__).parent.parent)

def cmd_build(args):
    for p in get_projects(): subprocess.run(["bash", "build.sh", "build"], cwd=p)

def cmd_release(args):
    central_dir = Path(__file__).parent.parent / "all_releases"; central_dir.mkdir(exist_ok=True)
    for p in get_projects(): 
        subprocess.run(["bash", "build.sh", "release"], cwd=p); proj_releases = p / "releases"
        if proj_releases.exists():
            for zf in proj_releases.glob("*.zip"): shutil.move(str(zf), str(central_dir / zf.name))
            shutil.rmtree(str(proj_releases), ignore_errors=True)
    from manifest_generator import generate_total_manifest; generate_total_manifest(Path(__file__).parent.parent)

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
        if args.offline: cmd.append("--offline")
        if args.dry_run: cmd.append("--dry-run")
        subprocess.run(cmd, cwd=p)

def cmd_update(args):
    setup = Path(__file__).parent.parent / "setup.py"
    for p in get_projects(): subprocess.run([sys.executable, str(setup.resolve())], cwd=p)

def cmd_generate_docs(args):
    gen = Path(__file__).parent / "doc_generator.py"
    for p in get_projects(): subprocess.run([sys.executable, str(gen), str(p)])

def cmd_generate_manifest(args):
    from manifest_generator import generate_total_manifest; generate_total_manifest(Path(__file__).parent.parent)

def cmd_workshop_tags(args):
    tags = Path(__file__).parent / "workshop_tags.txt"
    if tags.exists(): print(tags.read_text())

def cmd_workshop_info(args):
    auditor = Path(__file__).parent / "workshop_inspector.py"; subprocess.run([sys.executable, str(auditor)])

def main():
    # 1. Base Parser for Shared Flags
    base_parser = argparse.ArgumentParser(add_help=False)
    base_parser.add_argument("--json", action="store_true", help="Output results in JSON")
    base_parser.add_argument("--dry-run", action="store_true", help="Simulate actions")

    # 2. Main Parser
    parser = argparse.ArgumentParser(description="UKSF Taskforce Alpha Manager", parents=[base_parser])
    subparsers = parser.add_subparsers(dest="command")
    
    # Register common utilities
    simple_cmds = ["dashboard", "status", "build", "release", "test", "clean", "cache", "validate", "audit", "audit-updates", "apply-updates", "audit-deps", "audit-assets", "audit-strings", "audit-security", "audit-signatures", "audit-performance", "audit-keys", "generate-docs", "generate-manifest", "generate-preset", "generate-report", "generate-vscode", "generate-changelog", "setup-git-hooks", "check-env", "clean-strings", "update", "self-update", "workshop-tags", "gh-runs", "workshop-info", "help"]
    for cmd in simple_cmds:
        subparsers.add_parser(cmd, help=f"Run {cmd} utility", parents=[base_parser])
    
    p_lint = subparsers.add_parser("lint", help="Full Quality Lint", parents=[base_parser])
    p_lint.add_argument("--fix", action="store_true", help="Auto-fix formatting errors")

    p_ms = subparsers.add_parser("mission-setup", help="Standardize a mission folder", parents=[base_parser]); p_ms.add_argument("path", help="Path to mission folder"); p_ms.add_argument("--framework", action="store_true", help="Inject Mission Framework"); p_ms.epilog = "Example: ./tools/workspace_manager.py mission-setup my_op --framework"
    p_sync = subparsers.add_parser("sync", help="Synchronize mods", parents=[base_parser]); p_sync.add_argument("--offline", action="store_true")
    subparsers.add_parser("pull-mods", help="Alias for sync", parents=[base_parser]).add_argument("--offline", action="store_true")
    p_pub = subparsers.add_parser("publish", help="Upload to Steam", parents=[base_parser]); p_pub.add_argument("--dry-run-legacy", action="store_true")
    p_pub.add_argument("--offline", action="store_true", help="Generate local metadata only")
    p_conv = subparsers.add_parser("convert", help="Convert media", parents=[base_parser]); p_conv.add_argument("files", nargs="+")
    p_miss = subparsers.add_parser("audit-mission", help="Verify mission PBO", parents=[base_parser]); p_miss.add_argument("pbo")
    p_size = subparsers.add_parser("modlist-size", help="Calculate size", parents=[base_parser]); p_size.add_argument("file", nargs="?", default="mod_sources.txt")
    p_remote = subparsers.add_parser("remote", help="Distributed infrastructure management", parents=[base_parser]); p_remote.add_argument("cmd", nargs=argparse.REMAINDER)
    p_notify = subparsers.add_parser("notify", help="Discord update", parents=[base_parser]); p_notify.add_argument("message", nargs="?"); p_notify.add_argument("--type", choices=["update", "release", "alert"], default="update"); p_notify.add_argument("--title")
    p_class = subparsers.add_parser("classify-mod", help="Classify mod side requirement", parents=[base_parser]); p_class.add_argument("id", help="Steam Workshop ID or URL")
    p_list_class = subparsers.add_parser("modlist-classify", help="Classify entire modlist requirements", parents=[base_parser]); p_list_class.add_argument("file", nargs="?", default="mod_sources.txt", help="Path to modlist file")
    p_list_audit = subparsers.add_parser("modlist-audit", help="Compare modlist against reference sources", parents=[base_parser]); p_list_audit.add_argument("reference", help="Master HTML preset or TXT source"); p_list_audit.add_argument("targets", nargs="+", help="Sources to check against reference"); p_list_audit.add_argument("--deep", action="store_true", help="Scan dependencies of targets")
    
    p_fix = subparsers.add_parser("fix-syntax", help="Standardize indentation", parents=[base_parser])
    p_fix.add_argument("target", nargs="?", default=".", help="Target directory")

    args = parser.parse_args(); console = Console(force_terminal=True)
    cmds = {
        "dashboard": cmd_dashboard, "status": cmd_status, "sync": cmd_sync, "pull-mods": cmd_sync, "build": cmd_build, 
        "release": lambda a: subprocess.run([sys.executable, "tools/release.py"] + (["--dry-run"] if a.dry_run else [])), 
        "test": lambda a: subprocess.run(["pytest"]), "clean": lambda a: [subprocess.run(["rm", "-rf", ".hemttout"], cwd=p) for p in get_projects()],
        "cache": lambda a: [subprocess.run(["du", "-sh", ".hemttout"], cwd=p) for p in get_projects() if (p/".hemttout").exists()],
        "publish": cmd_publish, "audit": cmd_audit_full, "audit-updates": cmd_audit_updates, "apply-updates": cmd_apply_updates, "audit-deps": cmd_audit_deps,
        "audit-assets": cmd_audit_assets, "audit-strings": cmd_audit_strings, "audit-security": cmd_audit_security, "audit-signatures": cmd_audit_signatures,
        "audit-performance": cmd_audit_performance, "audit-keys": cmd_audit_keys, "audit-mission": cmd_audit_mission, "mission-setup": cmd_mission_setup, 
        "generate-docs": cmd_generate_docs, 
        "generate-manifest": lambda a: subprocess.run([sys.executable, "tools/manifest_generator.py"] + (["--dry-run"] if a.dry_run else [])), 
        "generate-preset": lambda a: subprocess.run([sys.executable, "tools/preset_generator.py"] + (["--dry-run"] if a.dry_run else [])), 
        "generate-report": lambda a: subprocess.run([sys.executable, "tools/report_generator.py"] + (["--dry-run"] if a.dry_run else [])), 
        "generate-vscode": cmd_generate_vscode, 
        "generate-changelog": lambda a: subprocess.run([sys.executable, "tools/changelog_generator.py"] + (["--dry-run"] if a.dry_run else [])), 
        "setup-git-hooks": cmd_setup_git_hooks,
        "check-env": cmd_check_env, 
        "fix-syntax": cmd_fix_syntax, 
        "clean-strings": cmd_clean_strings, "update": cmd_update, "self-update": cmd_self_update,
        "workshop-tags": cmd_workshop_tags, "gh-runs": cmd_gh_runs, "workshop-info": cmd_workshop_info, "classify-mod": cmd_classify_mod, "modlist-classify": cmd_modlist_classify, "modlist-audit": cmd_modlist_audit,
        "modlist-size": lambda a: subprocess.run([sys.executable, "tools/modlist_size.py", a.file]), 
        "notify": cmd_notify, 
        "convert": lambda a: [cmd_convert(a)], "help": lambda a: cmd_help(console),
        "lint": cmd_lint,
        "remote": lambda a: subprocess.run([sys.executable, "tools/remote_manager.py"] + (["--dry-run"] if a.dry_run else []) + a.cmd)
    }
    if args.command in cmds: cmds[args.command](args)
    else: cmd_help(console)

if __name__ == "__main__": main()
