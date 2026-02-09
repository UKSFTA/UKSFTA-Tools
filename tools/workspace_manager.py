#!/usr/bin/env python3
import argparse
import os
import subprocess
import re
import sys
import json
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
        def fit(text, title=None): return f"--- {title} ---\n{text}"

def get_projects():
    """Find all HEMTT projects in the parent directory."""
    parent_dir = Path(__file__).parent.parent.parent
    projects = []
    for d in parent_dir.iterdir():
        if d.is_dir() and (d / ".hemtt" / "project.toml").exists():
            projects.append(d)
    return sorted(projects)

def cmd_dashboard(args):
    console = Console(force_terminal=True)
    projects = get_projects()
    console.print(f"\n[bold green]UKSFTA Workspace Dashboard - {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}[/bold green]")
    
    table = Table(title=f"Total Projects: {len(projects)}", box=box.ROUNDED, header_style="bold magenta")
    table.add_column("Project", style="cyan", no_wrap=True)
    table.add_column("Prefix", style="magenta")
    table.add_column("Version", style="yellow")
    table.add_column("Workshop ID", style="green")
    table.add_column("Tags", style="blue")
    table.add_column("Status", justify="center")

    for p in projects:
        prefix = "Unknown"; version = "0.0.0"; ws_id = "None"; tags = []
        config_path = p / ".hemtt" / "project.toml"
        if config_path.exists():
            with open(config_path, 'r') as f:
                content = f.read()
                pm = re.search(r'prefix = "(.*)"', content)
                if pm: prefix = pm.group(1)
                wm = re.search(r'workshop_id = "(.*)"', content)
                if wm: ws_id = wm.group(1)
                tm = re.search(r'workshop_tags = \[(.*)\]', content)
                if tm: tags = [t.strip().replace('"', '').replace("'", "") for t in tm.group(1).split(',')]

        v_paths = [p / "addons" / x / "script_version.hpp" for x in ["main", "core", "maps", "zeus", "tmp", "temp"]]
        v_path = next((path for path in v_paths if path.exists()), None)
        if v_path:
            with open(v_path, 'r') as f:
                vc = f.read()
                ma = re.search(r'#define MAJOR (.*)', vc); mi = re.search(r'#define MINOR (.*)', vc); pa = re.search(r'#define PATCHLVL (.*)', vc)
                if ma and mi and pa: version = f"{ma.group(1).strip()}.{mi.group(1).strip()}.{pa.group(1).strip()}"

        table.add_row(p.name, prefix, version, ws_id, ", ".join(tags) if tags else "None", "[bold green]ONLINE[/bold green]")

    console.print(table)

def cmd_gh_runs(args):
    console = Console(force_terminal=True)
    projects = get_projects()
    table = Table(title="Organization CI/CD Status", box=box.ROUNDED, header_style="bold blue")
    table.add_column("Project", style="cyan", no_wrap=True)
    table.add_column("Workflow", style="magenta")
    table.add_column("Status", justify="center")
    table.add_column("Branch", style="dim")
    table.add_column("Message", style="italic")
    table.add_column("Age", justify="right")

    for p in projects:
        try:
            res = subprocess.run(["gh", "run", "list", "--limit", "1", "--json", "status,conclusion,workflowName,headBranch,displayTitle,createdAt"], cwd=p, capture_output=True, text=True)
            if res.returncode == 0:
                runs = json.loads(res.stdout)
                if not runs:
                    table.add_row(p.name, "-", "[dim]No Runs[/dim]", "-", "-", "-")
                    continue
                run = runs[0]; status_icon = "⚪"; status_style = "white"
                if run['status'] == "completed":
                    if run['conclusion'] == "success": status_icon = "✅ SUCCESS"; status_style = "bold green"
                    elif run['conclusion'] == "failure": status_icon = "❌ FAILED"; status_style = "bold red"
                    else: status_icon = f"❓ {run['conclusion'].upper()}"; status_style = "yellow"
                else: status_icon = "⏳ IN PROGRESS"; status_style = "bold cyan"
                
                created = datetime.fromisoformat(run['createdAt'].replace('Z', '+00:00'))
                diff = datetime.now(created.tzinfo) - created
                if diff.days > 0: age = f"{diff.days}d ago"
                elif diff.seconds > 3600: age = f"{diff.seconds // 3600}h ago"
                else: age = f"{diff.seconds // 60}m ago"

                table.add_row(p.name, run['workflowName'], f"[{status_style}]{status_icon}[/{status_style}]", run['headBranch'], run['displayTitle'][:40], age)
            else: table.add_row(p.name, "[red]Error[/red]", "Auth Fail", "-", "-", "-")
        except Exception: table.add_row(p.name, "[red]Failed[/red]", "-", "-", "-", "-")
    console.print(table)

def cmd_audit_deps(args):
    console = Console(force_terminal=True); projects = get_projects(); defined_patches = set(); dependencies = {}
    for p in projects:
        for config in p.glob("addons/*/config.cpp"):
            with open(config, 'r', errors='ignore') as f:
                content = f.read()
                for m in re.finditer(r'class\s+CfgPatches\s*\{[^}]*class\s+([a-zA-Z0-9_]+)', content, re.MULTILINE | re.DOTALL): defined_patches.add(m.group(1))
                rm = re.search(r'requiredAddons\[\]\s*=\s*\{([^}]*)\}', content, re.MULTILINE | re.DOTALL)
                if rm: dependencies[config] = [r.strip().replace('"', '').replace("'", "") for r in rm.group(1).split(',') if r.strip()]
    table = Table(title="Dependency Report", box=box.ROUNDED)
    table.add_column("File", style="dim"); table.add_column("Status", justify="center"); table.add_column("Missing", style="red")
    errs = 0; exts = ["A3_", "cba_", "ace_", "task_force_radio", "acre_", "rhsusf_", "rhs_"]
    for cfg, reqs in dependencies.items():
        rel = cfg.relative_to(Path(__file__).parent.parent.parent)
        miss = [r for r in reqs if r not in defined_patches and not any(r.lower().startswith(x.lower()) for x in exts)]
        if miss: table.add_row(str(rel), "[red]❌[/red]", ", ".join(miss)); errs += 1
        else: table.add_row(str(rel), "[green]✅[/green]", "")
    console.print(table)

def cmd_audit_assets(args):
    console = Console(force_terminal=True); projects = get_projects(); auditor = Path(__file__).parent / "asset_auditor.py"
    table = Table(title="Orphaned Assets", box=box.ROUNDED)
    table.add_column("Project", style="cyan"); table.add_column("Bloat Count", justify="right")
    for p in projects:
        res = subprocess.run([sys.executable, str(auditor), str(p)], capture_output=True, text=True)
        count = re.search(r'Found (\d+)', res.stdout).group(1) if "Found" in res.stdout else "0"
        table.add_row(p.name, count)
    console.print(table)

def cmd_audit_strings(args):
    console = Console(force_terminal=True); auditor = Path(__file__).parent / "string_auditor.py"
    table = Table(title="Localization Audit", box=box.ROUNDED)
    table.add_column("Project"); table.add_column("Status")
    for p in get_projects():
        res = subprocess.run([sys.executable, str(auditor), str(p)], capture_output=True, text=True)
        table.add_row(p.name, "[red]❌ ERR[/red]" if "MISSING" in res.stdout else "[green]✅ OK[/green]")
    console.print(table)

def cmd_status(args):
    for p in get_projects(): print(f"\n>>> {p.name}"); subprocess.run(["git", "status", "-s"], cwd=p)

def cmd_sync(args):
    for p in get_projects(): print(f"\n>>> Syncing {p.name}"); subprocess.run([sys.executable, "tools/manage_mods.py", "sync"], cwd=p)

def cmd_build(args):
    for p in get_projects(): print(f"\n>>> Building {p.name}"); subprocess.run(["bash", "build.sh", "build"], cwd=p)

def cmd_release(args):
    for p in get_projects(): print(f"\n>>> Release: {p.name}"); subprocess.run(["bash", "build.sh", "release"], cwd=p)

def cmd_publish(args):
    projects = get_projects(); publishable = []
    for p in projects:
        cp = p / ".hemtt" / "project.toml"
        if cp.exists():
            with open(cp, 'r') as f:
                c = f.read(); wm = re.search(r'workshop_id = "(.*)"', c)
                if wm and wm.group(1).isdigit(): publishable.append((p, wm.group(1)))
    for p, ws_id in publishable:
        cmd = [sys.executable, "tools/release.py", "-n", "-y"]
        if args.dry_run: cmd.append("--dry-run")
        subprocess.run(cmd, cwd=p)

def cmd_generate_docs(args):
    gen = Path(__file__).parent / "doc_generator.py"; p = Path(__file__).parent.parent.parent / "UKSFTA-Scripts"
    if p.exists(): subprocess.run([sys.executable, str(gen), str(p)])

def cmd_convert(args):
    from media_converter import convert_audio, convert_video, convert_image, check_ffmpeg, check_armake
    for f in args.files:
        ext = os.path.splitext(f)[1].lower()
        if ext in [".wav", ".mp3", ".m4a", ".flac"] and check_ffmpeg(): convert_audio(f)
        elif ext in [".mp4", ".mkv", ".mov", ".avi"] and check_ffmpeg(): convert_video(f)
        elif ext in [".png", ".jpg", ".jpeg"] and check_armake(): convert_image(f)

def cmd_update(args):
    setup = Path(__file__).parent.parent / "setup.py"
    for p in get_projects(): subprocess.run([sys.executable, str(setup.resolve())], cwd=p)

def cmd_workshop_tags(args):
    tags = Path(__file__).parent / "workshop_tags.txt"
    if tags.exists(): print(tags.read_text())

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Manager")
    subparsers = parser.add_subparsers(dest="command", help="Commands")
    for cmd in ["dashboard", "status", "sync", "build", "release", "test", "clean", "cache", "validate", "audit-deps", "audit-assets", "audit-strings", "generate-docs", "update", "workshop-tags", "gh-runs"]:
        subparsers.add_parser(cmd)
    p_pub = subparsers.add_parser("publish"); p_pub.add_argument("--dry-run", action="store_true")
    p_conv = subparsers.add_parser("convert"); p_conv.add_argument("files", nargs="+")
    args = parser.parse_args()
    cmds = {
        "dashboard": cmd_dashboard, "status": cmd_status, "sync": cmd_sync, "build": cmd_build, "release": cmd_release,
        "test": lambda a: subprocess.run(["pytest"]), "clean": lambda a: [subprocess.run(["rm", "-rf", ".hemttout"], cwd=p) for p in get_projects()],
        "cache": lambda a: [subprocess.run(["du", "-sh", ".hemttout"], cwd=p) for p in get_projects() if (p/".hemttout").exists()],
        "publish": cmd_publish, "audit-deps": cmd_audit_deps, "audit-assets": cmd_audit_assets, "audit-strings": cmd_audit_strings,
        "generate-docs": cmd_generate_docs, "update": cmd_update, "workshop-tags": cmd_workshop_tags, "gh-runs": cmd_gh_runs, "convert": cmd_convert
    }
    if args.command in cmds: cmds[args.command](args)
    else: parser.print_help()

if __name__ == "__main__":
    main()
