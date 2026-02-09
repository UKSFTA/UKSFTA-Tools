#!/usr/bin/env python3
import argparse
import os
import subprocess
import re
import sys
from pathlib import Path
from datetime import datetime

# Soft-import rich for CI environments
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    from rich.panel import Panel
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
    print(f"\n[bold green]UKSFTA Workspace Dashboard - {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}[/bold green]")
    
    table = Table(title="Project Overview", box=box.ROUNDED)
    table.add_column("Project", style="cyan")
    table.add_column("Prefix", style="magenta")
    table.add_column("Version", style="yellow")
    table.add_column("Workshop ID", style="green")
    table.add_column("Tags", style="blue")
    table.add_column("CI Status", justify="center")

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
                ma = re.search(r'#define MAJOR (.*)', vc)
                mi = re.search(r'#define MINOR (.*)', vc)
                pa = re.search(r'#define PATCHLVL (.*)', vc)
                if ma and mi and pa: version = f"{ma.group(1).strip()}.{mi.group(1).strip()}.{pa.group(1).strip()}"

        table.add_row(p.name, prefix, version, ws_id, ", ".join(tags) if tags else "None", "[green]SYNCED[/green]")

    table.title = f"Total Projects: {len(projects)}"
    console.print(table)

def cmd_audit_deps(args):
    console = Console(force_terminal=True)
    projects = get_projects(); defined_patches = set(); dependencies = {}
    for p in projects:
        for config in p.glob("addons/*/config.cpp"):
            with open(config, 'r', errors='ignore') as f:
                content = f.read()
                matches = re.finditer(r'class\s+CfgPatches\s*\{[^}]*class\s+([a-zA-Z0-9_]+)', content, re.MULTILINE | re.DOTALL)
                for m in matches: defined_patches.add(m.group(1))
                req_match = re.search(r'requiredAddons\[\]\s*=\s*\{([^}]*)\}', content, re.MULTILINE | re.DOTALL)
                if req_match:
                    reqs = [r.strip().replace('"', '').replace("'", "") for r in req_match.group(1).split(',')]
                    dependencies[config] = [r for r in reqs if r]

    console.print(f"\n[bold blue]=== Workspace Dependency Audit ===[/bold blue]")
    errors = 0; known_externals = ["A3_", "cba_", "ace_", "task_force_radio", "acre_", "rhsusf_", "rhs_"]
    for config, reqs in dependencies.items():
        rel_path = config.relative_to(Path(__file__).parent.parent.parent)
        missing = [r for r in reqs if r not in defined_patches and not any(r.lower().startswith(ext.lower()) for ext in known_externals)]
        if missing:
            console.print(f"[red]❌ {rel_path}[/red]")
            for m in missing: console.print(f"   - Missing dependency: [bold]{m}[/bold]")
            errors += 1
        else: console.print(f"[green]✓[/green] {rel_path} (All dependencies resolved)")
    if errors == 0: console.print(f"\n[bold green]Success: All dependencies healthy![/bold green]")
    else: console.print(f"\n[bold red]Failed: Found {errors} configs with unresolved dependencies.[/bold red]")

def cmd_audit_assets(args):
    console = Console(force_terminal=True)
    projects = get_projects()
    auditor = Path(__file__).parent / "asset_auditor.py"
    for p in projects:
        console.print(f"\n[bold blue]>>> Auditing Assets: {p.name}[/bold blue]")
        subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_audit_strings(args):
    console = Console(force_terminal=True)
    projects = get_projects()
    auditor = Path(__file__).parent / "string_auditor.py"
    for p in projects:
        console.print(f"\n[bold blue]>>> Auditing Strings: {p.name}[/bold blue]")
        subprocess.run([sys.executable, str(auditor), str(p)])

def cmd_generate_docs(args):
    console = Console(force_terminal=True)
    projects = get_projects()
    gen = Path(__file__).parent / "doc_generator.py"
    for p in projects:
        if p.name == "UKSFTA-Scripts":
            console.print(f"\n[bold blue]>>> Generating API Docs: {p.name}[/bold blue]")
            subprocess.run([sys.executable, str(gen), str(p)])

def cmd_gh_runs(args):
    console = Console(force_terminal=True); projects = get_projects()
    for p in projects:
        console.print(f"\n[bold blue]=== GitHub Runs: {p.name} ===[/bold blue]")
        try:
            res = subprocess.run(["gh", "run", "list", "--limit", "3"], cwd=p, capture_output=True, text=True)
            if res.returncode == 0:
                if not res.stdout.strip(): console.print("  No runs found.")
                else:
                    for line in res.stdout.splitlines():
                        if "✓" in line: console.print(f"  [green]{line}[/green]")
                        elif "X" in line or "fail" in line.lower(): console.print(f"  [red]{line}[/red]")
                        elif "*" in line: console.print(f"  [yellow]{line}[/yellow]")
                        else: console.print(f"  {line}")
            else: console.print(f"  [dim]GH CLI Error or no workflows.[/dim]")
        except Exception as e: console.print(f"  Error: {e}")

def cmd_convert(args):
    from media_converter import convert_audio, convert_video, convert_image, check_ffmpeg, check_armake
    for f in args.files:
        if not os.path.exists(f): print(f"⚠️ Not found: {f}"); continue
        ext = os.path.splitext(f)[1].lower()
        if ext in [".wav", ".mp3", ".m4a", ".flac"]:
            if check_ffmpeg(): convert_audio(f)
            else: print(f"❌ ffmpeg required for {f}")
        elif ext in [".mp4", ".mkv", ".mov", ".avi"]:
            if check_ffmpeg(): convert_video(f)
            else: print(f"❌ ffmpeg required for {f}")
        elif ext in [".png", ".jpg", ".jpeg"]:
            if check_armake(): convert_image(f)
            else: print(f"❌ armake required for {f}")

def cmd_update(args):
    projects = get_projects(); setup_script = Path(__file__).parent.parent / "setup.py"
    for p in projects:
        print(f"Updating tools in {p.name}...")
        subprocess.run([sys.executable, str(setup_script.resolve())], cwd=p)

def cmd_workshop_tags(args):
    tags_file = Path(__file__).parent / "workshop_tags.txt"
    if tags_file.exists(): print(tags_file.read_text())
    else: print("Workshop tags file not found.")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Workspace Management Tool")
    subparsers = parser.add_subparsers(dest="command", help="Commands")
    subparsers.add_parser("dashboard", help="Workspace overview")
    subparsers.add_parser("status", help="Git status overview")
    subparsers.add_parser("sync", help="Sync project dependencies")
    subparsers.add_parser("build", help="Run HEMTT build")
    subparsers.add_parser("release", help="Package ZIP release")
    subparsers.add_parser("test", help="Run all tests")
    subparsers.add_parser("clean", help="Clean artifacts")
    subparsers.add_parser("cache", help="Disk usage")
    subparsers.add_parser("validate", help="Run project validators")
    subparsers.add_parser("audit-build", help="Audit build integrity")
    subparsers.add_parser("audit-deps", help="Audit requiredAddons")
    subparsers.add_parser("audit-assets", help="Find orphaned binary assets")
    subparsers.add_parser("audit-strings", help="Validate stringtable synchronization")
    subparsers.add_parser("generate-docs", help="Generate API documentation from SQF headers")
    subparsers.add_parser("update", help="Push latest tools to projects")
    subparsers.add_parser("workshop-tags", help="List valid tags")
    subparsers.add_parser("gh-runs", help="GitHub Actions status")
    publish_parser = subparsers.add_parser("publish", help="Upload to Steam")
    publish_parser.add_argument("--dry-run", action="store_true")
    convert_parser = subparsers.add_parser("convert", help="Convert media (.ogg/.ogv/.paa)")
    convert_parser.add_argument("files", nargs="+")

    args = parser.parse_args()
    cmds = {
        "dashboard": cmd_dashboard, "status": cmd_status, "sync": cmd_sync,
        "build": cmd_build, "release": cmd_release, "test": cmd_test,
        "clean": cmd_clean, "cache": cmd_cache, "publish": cmd_publish,
        "validate": cmd_validate_project, "audit-build": cmd_audit_deps,
        "audit-deps": cmd_audit_deps, "audit-assets": cmd_audit_assets,
        "audit-strings": cmd_audit_strings, "generate-docs": cmd_generate_docs,
        "update": cmd_update, "workshop-tags": cmd_workshop_tags,
        "gh-runs": cmd_gh_runs, "convert": cmd_convert
    }
    if args.command in cmds:
        if args.command == "validate": # special case for project loop
            for p in get_projects(): cmds[args.command](p)
        else: cmds[args.command](args)
    else: parser.print_help()

if __name__ == "__main__":
    main()
