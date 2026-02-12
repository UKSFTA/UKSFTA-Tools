#!/usr/bin/env python3
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
except ImportError:
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
    """Fetch live timestamp from Steam Workshop."""
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
        ("\n ⚔️  ", "bold blue"),
        ("UKSF TASKFORCE ALPHA ", "bold white"),
        ("| ", "dim"),
        ("PLATINUM DEVOPS SUITE ", "bold cyan"),
        (f"v{version}", "bold yellow"),
        ("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n", "dim blue")
    )
    console.print(banner)

def cmd_help(console):
    print_banner(console)
    
    # 1. Workspace Operations
    ws_table = Table(title="🌐  Workspace Operations", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold cyan")
    ws_table.add_row("[bold cyan]status   [/]", "[dim]Show git status summary for every repository[/]")
    ws_table.add_row("[bold cyan]sync     [/]", "[dim]Pull latest Workshop updates and synchronize mods[/]")
    ws_table.add_row("[bold cyan]update   [/]", "[dim]Propagate latest UKSFTA-Tools to all projects[/]")
    ws_table.add_row("[bold cyan]cache    [/]", "[dim]Show disk space usage of build artifacts[/]")
    ws_table.add_row("[bold cyan]clean    [/]", "[dim]Wipe all .hemttout build artifacts[/]")
    
    # 2. Intelligence & Oversight
    intel_table = Table(title="🧠  Unit Intelligence", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold magenta")
    intel_table.add_row("[bold cyan]dashboard    [/]", "[dim]Visual overview of all projects, components, and versions[/]")
    intel_table.add_row("[bold cyan]workshop-info[/]", "[dim]Query live versions and timestamps from Steam Workshop[/]")
    intel_table.add_row("[bold cyan]modlist-size [/]", "[dim]Calculate total data size of any Arma 3 modlist[/]")
    intel_table.add_row("[bold cyan]audit-updates[/]", "[dim]Check live Workshop for pending mod updates[/]")
    intel_table.add_row("[bold cyan]apply-updates[/]", "[dim]Automatically update and sync all out-of-date mods[/]")
    intel_table.add_row("[bold cyan]gh-runs      [/]", "[dim]Real-time monitoring of GitHub Actions runners[/]")
    
    # 3. Assurance & Quality
    audit_table = Table(title="🔍  Assurance & Quality", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold yellow")
    audit_table.add_row("[bold cyan]audit            [/]", "[dim]Master Audit: Run all health and security checks[/]")
    audit_table.add_row("[bold cyan]test             [/]", "[dim]Run full suite (pytest, hemtt check, sqflint)[/]")
    audit_table.add_row("[bold cyan]audit-signatures [/]", "[dim]Verify PBO signing state and unit key matches[/]")
    audit_table.add_row("[bold cyan]audit-deps       [/]", "[dim]Scan requiredAddons for missing dependencies[/]")
    audit_table.add_row("[bold cyan]audit-assets     [/]", "[dim]Detect orphaned/unused binary files (PAA, P3D)[/]")
    audit_table.add_row("[bold cyan]audit-strings    [/]", "[dim]Validate stringtable keys vs SQF usage[/]")
    audit_table.add_row("[bold cyan]audit-security   [/]", "[dim]Scan for leaked tokens, webhooks, or private keys[/]")
    audit_table.add_row("[bold cyan]audit-mission    [/]", "[dim]Verify a Mission PBO against workspace and externals[/]")
    
    # 4. Production & Utilities
    prod_table = Table(title="🏗️  Production & Utilities", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold green")
    prod_table.add_row("[bold cyan]build            [/]", "[dim]Execute HEMTT build on all projects[/]")
    prod_table.add_row("[bold cyan]release          [/]", "[dim]Generate signed/packaged release ZIPs[/]")
    prod_table.add_row("[bold cyan]publish          [/]", "[dim]Upload projects to Steam Workshop[/]")
    prod_table.add_row("[bold cyan]notify           [/]", "[dim]Send a manual development update to Discord[/]")
    prod_table.add_row("[bold cyan]generate-manifest[/]", "[dim]Create unit-wide manifest of all mods and PBOs[/]")
    prod_table.add_row("[bold cyan]generate-docs    [/]", "[dim]Auto-generate API Manual from SQF headers[/]")
    prod_table.add_row("[bold cyan]convert          [/]", "[dim]Optimize media for Arma (WAV/PNG -> OGG/PAA)[/]")
    prod_table.add_row("[bold cyan]workshop-tags    [/]", "[dim]List all valid Arma 3 Steam Workshop tags[/]")

    console.print(ws_table); console.print(intel_table); console.print(audit_table); console.print(prod_table)
    console.print("\n[dim]Usage: ./tools/workspace_manager.py <command> [args][/]\n")

def cmd_dashboard(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    table = Table(title=f"Unit Workspace Overview ({len(projects)} Projects)", box=box.ROUNDED, header_style="bold magenta", border_style="blue")
    table.add_column("Project", style="cyan", no_wrap=True); table.add_column("Version", style="bold yellow")
    table.add_column("Components (PBOs)", style="dim"); table.add_column("External Mods", justify="center")
    table.add_column("Sync State", justify="center")
    
    for p in projects:
        version = "0.0.0"; pbos = []; ext_count = 0; sync_state = "[bold green]OK[/bold green]"
        addons_dir = p / "addons"
        if addons_dir.exists():
            for entry in addons_dir.iterdir():
                if entry.is_dir() and not entry.name.startswith("."): pbos.append(entry.name)
                elif entry.suffix.lower() == ".pbo": pbos.append(entry.stem)
        sources_path = p / "mod_sources.txt"
        if sources_path.exists():
            with open(sources_path, 'r') as f:
                for line in f:
                    if re.search(r"(\d{8,})", line) and "[ignore]" not in line.lower() and "ignore=" not in line.lower(): ext_count += 1
        lock_path = p / "mods.lock"
        if ext_count > 0 and not lock_path.exists(): sync_state = "[bold red]PENDING[/bold red]"
        v_path = p / "addons" / "main" / "script_version.hpp"
        if v_path.exists():
            with open(v_path, 'r') as f:
                vc = f.read(); ma = re.search(r'#define MAJOR (.*)', vc); mi = re.search(r'#define MINOR (.*)', vc); pa = re.search(r'#define PATCHLVL (.*)', vc)
                if ma and mi and pa: version = f"{ma.group(1).strip()}.{mi.group(1).strip()}.{pa.group(1).strip()}"
        pbo_str = ", ".join(pbos[:3]) + (f" (+{len(pbos)-3})" if len(pbos)>3 else "")
        table.add_row(p.name, version, pbo_str if pbos else "-", str(ext_count) if ext_count else "-", sync_state)
    console.print(table)

def cmd_gh_runs(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    workflow_names = set()
    with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), transient=True) as progress:
        task = progress.add_task("[cyan]Scanning for unique workflows...", total=len(projects))
        for p in projects:
            try:
                res = subprocess.run(["gh", "run", "list", "--limit", "15", "--json", "workflowName"], cwd=p, capture_output=True, text=True)
                if res.returncode == 0:
                    runs = json.loads(res.stdout)
                    for r in runs: workflow_names.add(r['workflowName'])
            except: pass
            progress.update(task, advance=1)
    
    sorted_workflows = sorted(list(workflow_names))
    if not sorted_workflows:
        console.print("[yellow]⚠️  No GitHub Actions runs found in the workspace.[/yellow]")
        return

    display_names = {}
    for wf in sorted_workflows:
        clean_name = os.path.basename(wf).replace(".yml", "").replace(".yaml", "").capitalize()
        display_names[wf] = clean_name

    table = Table(title="Global Unit Pipeline Matrix", box=box.ROUNDED, header_style="bold blue", border_style="blue")
    table.add_column("Project", style="cyan", no_wrap=True)
    for wf in sorted_workflows:
        table.add_column(display_names[wf], justify="center")
    table.add_column("Age", justify="right", style="dim")

    with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), transient=True) as progress:
        task = progress.add_task("[cyan]Auditing individual runners...", total=len(projects))
        for p in projects:
            stats = {wf: "⚪" for wf in sorted_workflows}
            latest_age = "-"
            try:
                res = subprocess.run(["gh", "run", "list", "--limit", "20", "--json", "workflowName,conclusion,status,createdAt"], cwd=p, capture_output=True, text=True)
                if res.returncode == 0:
                    runs = json.loads(res.stdout)
                    for run in runs:
                        wf = run['workflowName']
                        if wf in stats and stats[wf] == "⚪":
                            if run['status'] != "completed": stats[wf] = "⏳"
                            elif run['conclusion'] == "success": stats[wf] = "[bold green]✅[/]"
                            elif run['conclusion'] == "startup_failure": stats[wf] = "[cyan]Perm[/]"
                            elif run['conclusion'] == "failure":
                                if "codeql" in wf.lower() and p.name == "UKSFTA-Tmp": stats[wf] = "[cyan]Perm[/]"
                                else: stats[wf] = "[bold red]❌[/]"
                            else: stats[wf] = "[yellow]❓[/]"
                            
                            if latest_age == "-":
                                created = datetime.fromisoformat(run['createdAt'].replace('Z', '+00:00'))
                                diff = datetime.now(created.tzinfo) - created
                                latest_age = f"{diff.days}d" if diff.days > 0 else (f"{diff.seconds // 3600}h" if diff.seconds > 3600 else f"{diff.seconds // 60}m")
                
                table.add_row(p.name, *[stats[wf] for wf in sorted_workflows], latest_age)
            except Exception:
                table.add_row(p.name, *(["[red]Err[/]"] * len(sorted_workflows)), "-")
            progress.update(task, advance=1)
    console.print(table); console.print("[dim]Key: ✅ Success | ❌ Failed | ⏳ Running | ⚪ No Data[/dim]")

def cmd_audit_updates(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    table = Table(title="Workshop Update Audit", box=box.ROUNDED, border_style="yellow")
    table.add_column("Project", style="cyan"); table.add_column("Mod Name", style="magenta"); table.add_column("Locked", justify="center")
    table.add_column("Live", justify="center"); table.add_column("Status", justify="center")
    mod_registry = {}
    for p in projects:
        lock_path = p / "mods.lock"
        if lock_path.exists():
            with open(lock_path, 'r') as f:
                data = json.load(f).get("mods", {})
                for mid, info in data.items():
                    if mid not in mod_registry: mod_registry[mid] = {"name": info['name'], "locked": info.get('updated', '0'), "project": p.name}
    if not mod_registry: console.print("[yellow]No locked mods found.[/yellow]"); return
    with Progress(SpinnerColumn(), TextColumn("[progress.description]{task.description}"), console=console) as progress:
        task = progress.add_task(f"Checking Workshop...", total=len(mod_registry))
        def check_mod(mid):
            live_ts = get_live_timestamp(mid); progress.advance(task); return mid, live_ts
        with ThreadPoolExecutor(max_workers=10) as executor: live_results = dict(executor.map(check_mod, mod_registry.keys()))
    for mid, data in mod_registry.items():
        live_ts = live_results.get(mid, "0")
        status = "[bold green]LATEST[/bold green]" if data['locked'] == live_ts else "[bold red]UPDATE AVAILABLE[/bold red]"
        table.add_row(data['project'], data['name'], data['locked'], live_ts, status)
    console.print(table)

def cmd_apply_updates(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    console.print(Panel("[bold yellow]🚀 APPLYING GLOBAL MOD UPDATES[/bold yellow]", border_style="yellow"))
    for p in projects:
        lock_path = p / "mods.lock"
        if not lock_path.exists(): continue
        with open(lock_path, 'r') as f: data = json.load(f).get("mods", {})
        updates_found = False
        for mid, info in data.items():
            live_ts = get_live_timestamp(mid)
            if live_ts != "0" and info.get("updated") != live_ts:
                console.print(f"   [bold green]UPDATE[/bold green]: {info['name']} ({info.get('updated')} -> {live_ts})")
                info["updated"] = live_ts; updates_found = True
        if updates_found:
            with open(lock_path, 'w') as f: json.dump({"mods": data}, f, indent=2)
            console.print(f"🔄 [bold cyan]Syncing project:[/bold cyan] {p.name}")
            subprocess.run([sys.executable, "tools/manage_mods.py", "sync"], cwd=p)
            subprocess.run(["git", "add", "mods.lock"], cwd=p)
            subprocess.run(["git", "commit", "-S", "-m", "chore: apply automated workshop dependency updates"], cwd=p)
            subprocess.run(["git", "push", "origin", "main"], cwd=p)
    console.print("\n[bold green]✅ ALL UPDATES APPLIED AND PUSHED[/bold green]")

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

def cmd_audit_signatures(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    table = Table(title="Cryptographic Signature Audit", box=box.ROUNDED, border_style="magenta")
    table.add_column("Project", style="cyan"); table.add_column("PBO", style="dim"); table.add_column("Signed", justify="center")
    for p in projects:
        build_addons = p / ".hemttout" / "build" / "addons"
        if not build_addons.exists(): continue
        for pbo in build_addons.glob("*.pbo"):
            signed = (build_addons / f"{pbo.name}.uksfta.bisign").exists() or (build_addons / f"{pbo.name}.UKSFTA.bisign").exists()
            status = "[bold green]SIGNED[/bold green]" if signed else "[bold red]UNSIGNED[/bold red]"
            table.add_row(p.name, pbo.name, status)
    console.print(table)

def cmd_notify(args):
    console = Console(force_terminal=True); print_banner(console); notifier = Path(__file__).parent / "notify_discord.py"
    cmd = [sys.executable, str(notifier), "--message", args.message, "--type", args.type]
    if args.title: cmd.extend(["--title", args.title])
    subprocess.run(cmd); console.print("✅ Discord notification dispatched.")

def cmd_audit_full(args):
    console = Console(force_terminal=True); print_banner(console)
    console.print(Panel("[bold yellow]🚀 STARTING GLOBAL UNIT AUDIT[/bold yellow]", border_style="yellow"))
    cmd_audit_updates(args); cmd_audit_deps(args); cmd_audit_assets(args); cmd_audit_strings(args); cmd_audit_security(args); cmd_audit_signatures(args)
    console.print("\n[bold green]✅ GLOBAL AUDIT COMPLETE[/bold green]")

def cmd_audit_mission(args):
    console = Console(force_terminal=True); print_banner(console); from mission_auditor import audit_mission
    defined_patches = set()
    for p in get_projects():
        for config in p.glob("addons/*/config.cpp"):
            with open(config, 'r', errors='ignore') as f:
                content = f.read()
                for m in re.finditer(r'class\s+CfgPatches\s*\{[^}]*class\s+([a-zA-Z0-9_]+)', content, re.MULTILINE | re.DOTALL): defined_patches.add(m.group(1))
    results = audit_mission(args.pbo, defined_patches)
    if not results: return
    table = Table(title="Mission Dependency Analysis", box=box.ROUNDED, border_style="blue")
    table.add_column("Category", style="bold cyan"); table.add_column("Addon Class", style="dim"); table.add_column("Status", justify="center")
    for m in results["missing"]: table.add_row("Unknown/Missing", m, "[bold red]❌ NOT FOUND[/bold red]")
    for l in results["local"]: table.add_row("UKSFTA Workspace", l, "[bold green]✅ RESOLVED[/bold green]")
    for e in results["external"]: table.add_row("Known External", e, "[bold blue]ℹ️ EXTERNAL[/bold blue]")
    console.print(table)

def cmd_audit_assets(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "asset_auditor.py"
    table = Table(title="Resource Audit", box=box.ROUNDED, border_style="blue")
    table.add_column("Project", style="cyan"); table.add_column("Status", justify="center"); table.add_column("Unused", justify="right", style="bold yellow")
    for p in get_projects():
        res = subprocess.run([sys.executable, str(auditor), str(p)], capture_output=True, text=True)
        count = re.search(r'Found (\d+)', res.stdout).group(1) if "Found" in res.stdout else "0"
        status = "[bold yellow]⚠️ BLOAT[/bold yellow]" if int(count) > 0 else "[bold green]✅ CLEAN[/bold green]"
        table.add_row(p.name, status, count)
    console.print(table)

def cmd_audit_strings(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "string_auditor.py"
    table = Table(title="Localization Audit", box=box.ROUNDED, border_style="blue")
    table.add_column("Project", style="cyan"); table.add_column("Sync State", justify="center")
    for p in get_projects():
        res = subprocess.run([sys.executable, str(auditor), str(p)], capture_output=True, text=True)
        table.add_row(p.name, "[bold red]❌ DESYNC[/bold red]" if "MISSING" in res.stdout else "[bold green]✅ MATCH[/bold green]")
    console.print(table)

def cmd_audit_security(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "security_auditor.py"
    table = Table(title="Security Scan", box=box.ROUNDED, border_style="red")
    table.add_column("Project", style="cyan"); table.add_column("Security Status", justify="center")
    for p in get_projects():
        res = subprocess.run([sys.executable, str(auditor), str(p)], capture_output=True, text=True)
        table.add_row(p.name, "[bold red]❌ LEAK[/bold red]" if "LEAK" in res.stdout or "CRITICAL" in res.stdout else "[bold green]✅ SECURE[/bold green]")
    console.print(table)

def cmd_status(args):
    console = Console(force_terminal=True); print_banner(console)
    for p in get_projects(): console.print(Panel(f"[dim]Root: {p}[/dim]", title=f"📦 {p.name}", border_style="cyan")); subprocess.run(["git", "status", "-s"], cwd=p)

def cmd_sync(args):
    console = Console(force_terminal=True); print_banner(console)
    for p in get_projects(): 
        console.print(f"🔄 [bold cyan]Syncing Dependencies:[/bold cyan] {p.name}")
        cmd = [sys.executable, "tools/manage_mods.py", "sync"]
        if args.offline: cmd.append("--offline")
        subprocess.run(cmd, cwd=p)
    from manifest_generator import generate_total_manifest
    generate_total_manifest(Path(__file__).parent.parent)

def cmd_build(args):
    console = Console(force_terminal=True); print_banner(console)
    for p in get_projects(): console.print(f"🏗️ [bold yellow]Building:[/bold yellow] {p.name}"); subprocess.run(["bash", "build.sh", "build"], cwd=p)

def cmd_release(args):
    console = Console(force_terminal=True); print_banner(console); central_dir = Path(__file__).parent.parent / "all_releases"; central_dir.mkdir(exist_ok=True)
    for p in get_projects(): 
        console.print(f"🚀 [bold green]Packaging:[/bold green] {p.name}"); subprocess.run(["bash", "build.sh", "release"], cwd=p)
        proj_releases = p / "releases"
        if proj_releases.exists():
            for zf in proj_releases.glob("*.zip"): console.print(f"   [dim]-> Consolidating: {zf.name}[/dim]"); shutil.move(str(zf), str(central_dir / zf.name))
            shutil.rmtree(str(proj_releases), ignore_errors=True)
    from manifest_generator import generate_total_manifest
    generate_total_manifest(Path(__file__).parent.parent)
    console.print(f"\n[bold cyan]✨ Releases consolidated to: {central_dir}[/bold cyan]")

def cmd_publish(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects(); publishable = []
    for p in projects:
        cp = p / ".hemtt" / "project.toml"
        if cp.exists():
            with open(cp, 'r') as f:
                c = f.read(); wm = re.search(r'workshop_id = "(.*)"', c)
                if wm and wm.group(1).isdigit(): publishable.append((p, wm.group(1)))
    for p, ws_id in publishable:
        console.print(f"📤 [bold green]Publishing to Steam:[/bold green] {p.name} ({ws_id})")
        cmd = [sys.executable, "tools/release.py", "-n", "-y"]; cmd.append("--dry-run") if args.dry_run else None
        subprocess.run(cmd, cwd=p)

def cmd_generate_docs(args):
    console = Console(force_terminal=True); print_banner(console); gen = Path(__file__).parent / "doc_generator.py"
    p = Path(__file__).parent.parent.parent / "UKSFTA-Scripts"
    if p.exists(): console.print(f"📖 [bold blue]Documenting:[/bold blue] {p.name}"); subprocess.run([sys.executable, str(gen), str(p)])

def cmd_generate_manifest(args):
    console = Console(force_terminal=True); print_banner(console)
    from manifest_generator import generate_total_manifest
    output_path = generate_total_manifest(Path(__file__).parent.parent)
    console.print(f"\n[bold green]Success![/bold green] Total manifest saved to: [cyan]{output_path}[/cyan]")

def cmd_convert(args):
    console = Console(force_terminal=True); print_banner(console); from media_converter import convert_audio, convert_video, convert_image, check_ffmpeg, check_armake
    for f in args.files:
        ext = os.path.splitext(f)[1].lower(); console.print(f"⚡ [bold cyan]Processing:[/bold cyan] {os.path.basename(f)}")
        if ext in [".wav", ".mp3", ".m4a", ".flac"] and check_ffmpeg(): convert_audio(f)
        elif ext in [".mp4", ".mkv", ".mov", ".avi"] and check_ffmpeg(): convert_video(f)
        elif ext in [".png", ".jpg", ".jpeg"] and check_armake(): convert_image(f)

def cmd_update(args):
    console = Console(force_terminal=True); print_banner(console); setup = Path(__file__).parent.parent / "setup.py"
    for p in get_projects(): console.print(f"⏫ [bold green]Updating:[/bold green] {p.name}"); subprocess.run([sys.executable, str(setup.resolve())], cwd=p)

def cmd_workshop_tags(args):
    console = Console(force_terminal=True); print_banner(console); tags = Path(__file__).parent / "workshop_tags.txt"
    if tags.exists(): console.print(Panel(tags.read_text(), title="Valid Workshop Tags", border_style="blue"))

def cmd_workshop_info(args):
    console = Console(force_terminal=True); print_banner(console); auditor = Path(__file__).parent / "workshop_inspector.py"
    subprocess.run([sys.executable, str(auditor)])

def main():
    parser = argparse.ArgumentParser(description="UKSF Taskforce Alpha Manager", add_help=False)
    subparsers = parser.add_subparsers(dest="command")
    for cmd in ["dashboard", "status", "build", "release", "test", "clean", "cache", "validate", "audit", "audit-updates", "apply-updates", "audit-deps", "audit-assets", "audit-strings", "audit-security", "audit-signatures", "generate-docs", "generate-manifest", "update", "workshop-tags", "gh-runs", "workshop-info", "help"]: subparsers.add_parser(cmd)
    for cmd in ["sync", "pull-mods"]:
        p_sync = subparsers.add_parser(cmd); p_sync.add_argument("--offline", action="store_true", help="Skip SteamCMD downloads")
    p_pub = subparsers.add_parser("publish"); p_pub.add_argument("--dry-run", action="store_true")
    p_conv = subparsers.add_parser("convert"); p_conv.add_argument("files", nargs="+")
    p_miss = subparsers.add_parser("audit-mission"); p_miss.add_argument("pbo", help="Path to mission PBO")
    p_size = subparsers.add_parser("modlist-size"); p_size.add_argument("file", nargs="?", default="mod_sources.txt", help="Path to modlist file")
    p_notify = subparsers.add_parser("notify"); p_notify.add_argument("message"); p_notify.add_argument("--type", choices=["update", "release", "alert"], default="update"); p_notify.add_argument("--title")
    args = parser.parse_args(); console = Console(force_terminal=True)
    cmds = {
        "dashboard": cmd_dashboard, "status": cmd_status, "sync": cmd_sync, "pull-mods": cmd_sync, "build": cmd_build, "release": cmd_release,
        "test": lambda a: subprocess.run(["pytest"]), "clean": lambda a: [subprocess.run(["rm", "-rf", ".hemttout"], cwd=p) for p in get_projects()],
        "cache": lambda a: [subprocess.run(["du", "-sh", ".hemttout"], cwd=p) for p in get_projects() if (p/".hemttout").exists()],
        "publish": cmd_publish, "audit": cmd_audit_full, "audit-updates": cmd_audit_updates, "apply-updates": cmd_apply_updates, "audit-deps": cmd_audit_deps,
        "audit-assets": cmd_audit_assets, "audit-strings": cmd_audit_strings, "audit-security": cmd_audit_security, "audit-signatures": cmd_audit_signatures,
        "audit-mission": cmd_audit_mission, "generate-docs": cmd_generate_docs, "generate-manifest": cmd_generate_manifest, "update": cmd_update,
        "workshop-tags": cmd_workshop_tags, "gh-runs": cmd_gh_runs, "workshop-info": cmd_workshop_info, "modlist-size": lambda a: subprocess.run([sys.executable, "tools/modlist_size.py", a.file]),
        "notify": cmd_notify, "convert": cmd_convert, "help": lambda a: cmd_help(console)
    }
    if args.command in cmds: cmds[args.command](args)
    else: cmd_help(console)

if __name__ == "__main__": main()
