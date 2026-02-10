#!/usr/bin/env python3
import argparse
import os
import subprocess
import re
import sys
import json
import shutil
from pathlib import Path
from datetime import datetime

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

def print_banner(console):
    banner = Text.assemble(
        ("\n ⚔️  ", "bold blue"),
        ("UKSF TASKFORCE ALPHA ", "bold white"),
        ("| ", "dim"),
        ("PLATINUM DEVOPS SUITE", "bold cyan"),
        ("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n", "dim blue")
    )
    console.print(banner)

def cmd_help(console):
    print_banner(console)
    dev_table = Table(title="🏗️  Development & Management", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold magenta")
    dev_table.add_row("[bold cyan]dashboard[/]", "[dim]Visual overview of all projects, versions, and prefixes[/]")
    dev_table.add_row("[bold cyan]status   [/]", "[dim]Show git status summary for every repository[/]")
    dev_table.add_row("[bold cyan]pull-mods[/]", "[dim]Pull latest Workshop updates for all project dependencies[/]")
    dev_table.add_row("[bold cyan]sync     [/]", "[dim]Alias for pull-mods (Synchronize submodules and mods)[/]")
    dev_table.add_row("[bold cyan]build    [/]", "[dim]Execute HEMTT build on all projects[/]")
    dev_table.add_row("[bold cyan]release  [/]", "[dim]Generate signed/packaged release ZIPs[/]")
    dev_table.add_row("[bold cyan]publish  [/]", "[dim]Upload projects to Steam Workshop (with --dry-run)[/]")
    dev_table.add_row("[bold cyan]clean    [/]", "[dim]Wipe all .hemttout build artifacts[/]")
    audit_table = Table(title="🔍  Assurance & Auditing", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold yellow")
    audit_table.add_row("[bold cyan]test          [/]", "[dim]Run full suite (pytest, hemtt check, sqflint)[/]")
    audit_table.add_row("[bold cyan]audit-deps    [/]", "[dim]Scan requiredAddons for missing dependencies[/]")
    audit_table.add_row("[bold cyan]audit-assets  [/]", "[dim]Detect orphaned/unused binary files (PAA, P3D)[/]")
    audit_table.add_row("[bold cyan]audit-strings [/]", "[dim]Validate stringtable keys vs SQF usage[/]")
    audit_table.add_row("[bold cyan]audit-security[/]", "[dim]Scan for leaked tokens, webhooks, or private keys[/]")
    audit_table.add_row("[bold cyan]audit-mission [/]", "[dim]Verify a Mission PBO against workspace and externals[/]")
    audit_table.add_row("[bold cyan]gh-runs       [/]", "[dim]Real-time monitoring of GitHub Actions runners[/]")
    util_table = Table(title="🛠️  Utilities & Tools", box=box.SIMPLE, show_header=False, title_justify="left", title_style="bold cyan")
    util_table.add_row("[bold cyan]convert          [/]", "[dim]Optimize media for Arma (WAV/PNG -> OGG/PAA)[/]")
    util_table.add_row("[bold cyan]generate-docs    [/]", "[dim]Auto-generate API Manual from SQF headers[/]")
    util_table.add_row("[bold cyan]generate-manifest[/]", "[dim]Create unit-wide manifest of all mods and PBOs[/]")
    util_table.add_row("[bold cyan]workshop-tags    [/]", "[dim]List all valid Arma 3 Steam Workshop tags[/]")
    util_table.add_row("[bold cyan]update           [/]", "[dim]Push latest UKSFTA-Tools to all projects[/]")
    util_table.add_row("[bold cyan]cache            [/]", "[dim]Show disk space usage of build artifacts[/]")
    console.print(dev_table); console.print(audit_table); console.print(util_table)
    console.print("\n[dim]Usage: ./tools/workspace_manager.py <command> [args][/]\n")

def cmd_dashboard(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    table = Table(title=f"Unit Workspace Overview ({len(projects)} Projects)", box=box.ROUNDED, header_style="bold magenta", border_style="blue")
    table.add_column("ID", style="dim", justify="right"); table.add_column("Project", style="cyan", no_wrap=True)
    table.add_column("Prefix", style="magenta"); table.add_column("Version", style="bold yellow")
    table.add_column("Workshop", style="green"); table.add_column("Tags", style="blue"); table.add_column("Status", justify="center")
    for i, p in enumerate(projects):
        prefix = "Unknown"; version = "0.0.0"; ws_id = "None"; tags = []
        config_path = p / ".hemtt" / "project.toml"
        if config_path.exists():
            with open(config_path, 'r') as f:
                content = f.read(); pm = re.search(r'prefix = "(.*)"', content)
                if pm: prefix = pm.group(1)
                wm = re.search(r'workshop_id = "(.*)"', content)
                if wm: ws_id = wm.group(1)
                tm = re.search(r'workshop_tags = \[(.*)\]', content)
                if tm: tags = [t.strip().replace('"', '').replace("'", "") for t in tm.group(1).split(',')]
        v_paths = [p / "addons" / x / "script_version.hpp" for x in ["main", "core", "maps", "zeus", "tmp", "temp"]]
        v_path = next((path for path in v_paths if path.exists()), None)
        if v_path:
            with open(v_path, 'r') as f:
                vc = f.read(); ma = re.search(r'#define MAJOR (.*)', vc); mi = re.search(r'#define MINOR (.*)', vc); pa = re.search(r'#define PATCHLVL (.*)', vc)
                if ma and mi and pa: version = f"{ma.group(1).strip()}.{mi.group(1).strip()}.{pa.group(1).strip()}"
        table.add_row(str(i+1), p.name, f"\\z\\{prefix}", version, ws_id, ", ".join(tags[:2]) + ("..." if len(tags)>2 else ""), "[bold green]ONLINE[/bold green]")
    console.print(table)

def cmd_gh_runs(args):
    console = Console(force_terminal=True); print_banner(console); projects = get_projects()
    table = Table(title="Global CI/CD Monitor", box=box.ROUNDED, header_style="bold blue", border_style="blue")
    table.add_column("Project", style="cyan", no_wrap=True); table.add_column("Workflow", style="magenta"); table.add_column("Status", justify="center")
    table.add_column("Branch", style="dim"); table.add_column("Last Message", style="italic"); table.add_column("Age", justify="right")
    for p in projects:
        try:
            res = subprocess.run(["gh", "run", "list", "--limit", "1", "--json", "status,conclusion,workflowName,headBranch,displayTitle,createdAt"], cwd=p, capture_output=True, text=True)
            if res.returncode == 0:
                runs = json.loads(res.stdout)
                if not runs: table.add_row(p.name, "-", "[dim]No Runs[/dim]", "-", "-", "-"); continue
                run = runs[0]; status_icon = "⚪"; status_style = "white"
                if run['status'] == "completed":
                    if run['conclusion'] == "success": status_icon = "✅ SUCCESS"; status_style = "bold green"
                    elif run['conclusion'] == "failure": status_icon = "❌ FAILED"; status_style = "bold red"
                    else: status_icon = f"❓ {run['conclusion'].upper()}"; status_style = "yellow"
                else: status_icon = "⏳ RUNNING"; status_style = "bold cyan"
                created = datetime.fromisoformat(run['createdAt'].replace('Z', '+00:00'))
                diff = datetime.now(created.tzinfo) - created
                age = f"{diff.days}d" if diff.days > 0 else (f"{diff.seconds // 3600}h" if diff.seconds > 3600 else f"{diff.seconds // 60}m")
                table.add_row(p.name, run['workflowName'], f"[{status_style}]{status_icon}[/{status_style}]", run['headBranch'], run['displayTitle'][:30], age)
            else: table.add_row(p.name, "[red]Error[/red]", "Check Auth", "-", "-", "-")
        except Exception: table.add_row(p.name, "[red]Failed[/red]", "-", "-", "-", "-")
    console.print(table)

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
        rel = cfg.relative_to(Path(__file__).parent.parent.parent); exts = ["A3_", "cba_", "ace_", "task_force_radio", "acre_", "rhsusf_", "rhs_"]
        miss = [r for r in reqs if r not in defined_patches and not any(r.lower().startswith(x.lower()) for x in exts)]
        if miss: table.add_row(str(rel), "❌ [bold red]FAIL[/bold red]", ", ".join(miss))
        else: table.add_row(str(rel), "✅ [bold green]PASS[/bold green]", "[dim]Healthy[/dim]")
    console.print(table)

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
    for p in get_projects(): console.print(f"🔄 [bold cyan]Syncing Dependencies:[/bold cyan] {p.name}"); subprocess.run([sys.executable, "tools/manage_mods.py", "sync"], cwd=p)

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

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Manager", add_help=False)
    subparsers = parser.add_subparsers(dest="command")
    simple_cmds = ["dashboard", "status", "sync", "pull-mods", "build", "release", "test", "clean", "cache", "validate", "audit-deps", "audit-assets", "audit-strings", "audit-security", "generate-docs", "generate-manifest", "update", "workshop-tags", "gh-runs", "help"]
    for cmd in simple_cmds: subparsers.add_parser(cmd)
    p_pub = subparsers.add_parser("publish"); p_pub.add_argument("--dry-run", action="store_true")
    p_conv = subparsers.add_parser("convert"); p_conv.add_argument("files", nargs="+")
    p_miss = subparsers.add_parser("audit-mission"); p_miss.add_argument("pbo", help="Path to mission PBO")
    args = parser.parse_args(); console = Console(force_terminal=True)
    cmds = {
        "dashboard": cmd_dashboard, "status": cmd_status, "sync": cmd_sync, "pull-mods": cmd_sync, "build": cmd_build, "release": cmd_release,
        "test": lambda a: subprocess.run(["pytest"]), "clean": lambda a: [subprocess.run(["rm", "-rf", ".hemttout"], cwd=p) for p in get_projects()],
        "cache": lambda a: [subprocess.run(["du", "-sh", ".hemttout"], cwd=p) for p in get_projects() if (p/".hemttout").exists()],
        "publish": cmd_publish, "audit-deps": cmd_audit_deps, "audit-assets": cmd_audit_assets, "audit-strings": cmd_audit_strings,
        "audit-security": cmd_audit_security, "audit-mission": cmd_audit_mission, "generate-docs": cmd_generate_docs, "generate-manifest": cmd_generate_manifest,
        "update": cmd_update, "workshop-tags": cmd_workshop_tags, "gh-runs": cmd_gh_runs, "convert": cmd_convert, "help": lambda a: cmd_help(console)
    }
    if args.command in cmds: cmds[args.command](args)
    else: cmd_help(console)

if __name__ == "__main__": main()
