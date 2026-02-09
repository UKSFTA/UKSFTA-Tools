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
    # Minimal polyfill for environment without rich
    class Table:
        def __init__(self, **kwargs): self.rows = []
        def add_column(self, *args, **kwargs): pass
        def add_row(self, *args): self.rows.append(args)
    class Console:
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
        # Extract metadata
        prefix = "Unknown"
        version = "0.0.0"
        ws_id = "None"
        tags = []
        
        config_path = p / ".hemtt" / "project.toml"
        if config_path.exists():
            with open(config_path, 'r') as f:
                content = f.read()
                prefix_match = re.search(r'prefix = "(.*)"', content)
                if prefix_match: prefix = prefix_match.group(1)
                
                ws_id_match = re.search(r'workshop_id = "(.*)"', content)
                if ws_id_match: ws_id = ws_id_match.group(1)

                tags_match = re.search(r'workshop_tags = \[(.*)\]', content)
                if tags_match:
                    tags = [t.strip().replace('"', '').replace("'", "") for t in tags_match.group(1).split(',')]

        # Get version from script_version.hpp if possible
        v_paths = [
            p / "addons" / "main" / "script_version.hpp",
            p / "addons" / "core" / "script_version.hpp",
            p / "addons" / "maps" / "script_version.hpp",
            p / "addons" / "zeus" / "script_version.hpp",
            p / "addons" / "tmp" / "script_version.hpp",
            p / "addons" / "temp" / "script_version.hpp",
        ]
        
        v_path = next((path for path in v_paths if path.exists()), None)
            
        if v_path:
            with open(v_path, 'r') as f:
                v_content = f.read()
                major = re.search(r'#define MAJOR (.*)', v_content)
                minor = re.search(r'#define MINOR (.*)', v_content)
                patch = re.search(r'#define PATCHLVL (.*)', v_content)
                if major and minor and patch:
                    version = f"{major.group(1).strip()}.{minor.group(1).strip()}.{patch.group(1).strip()}"

        table.add_row(
            p.name,
            prefix,
            version,
            ws_id,
            ", ".join(tags) if tags else "None",
            "[green]SYNCED[/green]" 
        )

    console = Console()
    table.title = f"Total Projects: {len(projects)}"
    console.print(table)

def cmd_status(args):
    projects = get_projects()
    for p in projects:
        print(f"\n>>> Project: {p.name}")
        subprocess.run(["git", "status", "-s"], cwd=p)

def cmd_sync(args):
    projects = get_projects()
    for p in projects:
        print(f"\n>>> Syncing {p.name}...")
        subprocess.run([sys.executable, "tools/manage_mods.py", "sync"], cwd=p)

def cmd_build(args):
    projects = get_projects()
    for p in projects:
        print(f"\n>>> Building {p.name}...")
        subprocess.run(["bash", "build.sh", "build"], cwd=p)

def cmd_release(args):
    projects = get_projects()
    for p in projects:
        print(f"\n>>> Packaging Release: {p.name}...")
        subprocess.run(["bash", "build.sh", "release"], cwd=p)

def cmd_test(args):
    # 1. Run Python tool tests
    print("[bold cyan]Step 1: Running Python Tool Tests (pytest)[/bold cyan]")
    subprocess.run(["pytest"])

    # 2. Run Workspace-wide HEMTT and custom checks
    projects = get_projects()
    for p in projects:
        print(f"\n[bold blue]=== Testing Project: {p.name} ===[/bold blue]")
        
        print("  - Running HEMTT Check...")
        subprocess.run(["hemtt", "check"], cwd=p)
        
        print("  - Running SQFLint...")
        subprocess.run(["sqflint", "addons"], cwd=p)

        print("  - Running UKSFTA Validators...")
        cmd_validate_project(p)

def cmd_validate_project(p):
    tools_dir = Path(__file__).parent.resolve()
    validators = [
        "config_style_checker.py",
        "sqf_validator.py",
        "stringtable_validator.py",
        "return_checker.py",
        "search_unused_privates.py"
    ]
    for val in validators:
        val_path = tools_dir / val
        if val_path.exists():
            subprocess.run([sys.executable, str(val_path)], cwd=p)

def cmd_clean(args):
    projects = get_projects()
    for p in projects:
        out_dir = p / ".hemttout"
        if out_dir.exists():
            print(f"Cleaning {p.name}...")
            subprocess.run(["rm", "-rf", str(out_dir)])

def cmd_cache(args):
    projects = get_projects()
    for p in projects:
        out_dir = p / ".hemttout"
        if out_dir.exists():
            size = subprocess.check_output(["du", "-sh", str(out_dir)]).split()[0].decode('utf-8')
            print(f"  {p.name}: {size}")

def cmd_publish(args):
    projects = get_projects()
    publishable = []
    
    for p in projects:
        config_path = p / ".hemtt" / "project.toml"
        if config_path.exists():
            with open(config_path, 'r') as f:
                content = f.read()
                ws_id_match = re.search(r'workshop_id = "(.*)"', content)
                if ws_id_match and ws_id_match.group(1).isdigit():
                    publishable.append((p, ws_id_match.group(1)))

    if not publishable:
        print("No projects found with a valid Steam Workshop ID.")
        return

    if args.dry_run:
        print("\n[bold yellow]--- PUBLISH DRY-RUN MODE ---[/bold yellow]")
    else:
        mode_label = "PRODUCTION UPLOAD" if not args.dry_run else "DRY-RUN SIMULATION"
        print(f"\nTarget Projects for {mode_label}:")
        
    for p, ws_id in publishable:
        print(f"  - {p.name} (ID: {ws_id})")
    
    if not args.dry_run:
        confirm = input("\nProceed with publishing ALL identified projects? [y/N]: ").lower()
        if confirm != 'y':
            print("Aborting.")
            return

    for p, ws_id in publishable:
        print(f"\n>>> Publishing {p.name} to Workshop...")
        cmd = [sys.executable, "tools/release.py", "-n", "-y"]
        if args.dry_run:
            cmd.append("--dry-run")
        subprocess.run(cmd, cwd=p)

def cmd_validate(args):
    projects = get_projects()
    for p in projects:
        print(f"\n=== Validating {p.name} ===")
        cmd_validate_project(p)

def cmd_audit_build(args):
    projects = get_projects()
    checker = Path(__file__).parent / "mod_integrity_checker.py"
    for p in projects:
        build_path = p / ".hemttout" / "release"
        if build_path.exists():
            cmd = [sys.executable, str(checker.resolve()), str(build_path.resolve()), "--unsigned"]
            subprocess.run(cmd)
        else:
            print(f"\n>>> Skipping {p.name}: No built artifacts found in .hemttout/release")

def cmd_update(args):
    projects = get_projects()
    setup_script = Path(__file__).parent.parent / "setup.py"
    for p in projects:
        print(f"Updating tools in {p.name}...")
        subprocess.run([sys.executable, str(setup_script.resolve())], cwd=p)

def cmd_workshop_tags(args):
    tags_file = Path(__file__).parent / "workshop_tags.txt"
    if tags_file.exists():
        print(tags_file.read_text())
    else:
        print("Workshop tags reference file not found.")

def cmd_audit_deps(args):
    projects = get_projects()
    defined_patches = set()
    dependencies = {}

    # 1. Discover all patches defined in the workspace
    for p in projects:
        for config in p.glob("addons/*/config.cpp"):
            with open(config, 'r', errors='ignore') as f:
                content = f.read()
                # Find class CfgPatches { class NAME
                matches = re.finditer(r'class\s+CfgPatches\s*\{[^}]*class\s+([a-zA-Z0-9_]+)', content, re.MULTILINE | re.DOTALL)
                for m in matches:
                    patch_name = m.group(1)
                    defined_patches.add(patch_name)
                    
                # Find requiredAddons[] = { "A", "B" };
                req_match = re.search(r'requiredAddons\[\]\s*=\s*\{([^}]*)\}', content, re.MULTILINE | re.DOTALL)
                if req_match:
                    reqs = [r.strip().replace('"', '').replace("'", "") for r in req_match.group(1).split(',')]
                    reqs = [r for r in reqs if r] # Filter empty
                    dependencies[config] = reqs

    # 2. Validate
    print(f"\n[bold blue]=== Workspace Dependency Audit ===[/bold blue]")
    print(f"Found {len(defined_patches)} local patches defined.\n")
    
    errors = 0
    known_externals = ["A3_", "cba_", "ace_", "task_force_radio", "acre_", "rhsusf_", "rhs_"]

    for config, reqs in dependencies.items():
        rel_path = config.relative_to(Path(__file__).parent.parent.parent)
        missing = []
        for r in reqs:
            if r in defined_patches:
                continue
            if any(r.lower().startswith(ext.lower()) for ext in known_externals):
                continue
            missing.append(r)
        
        if missing:
            print(f"[red]❌ {rel_path}[/red]")
            for m in missing:
                print(f"   - Missing dependency: [bold]{m}[/bold]")
            errors += 1
        else:
            print(f"[green]✓[/green] {rel_path} (All dependencies resolved)")

    if errors == 0:
        print(f"\n[bold green]Success: All workspace dependencies are healthy![/bold green]")
    else:
        print(f"\n[bold red]Failed: Found {errors} configs with unresolved dependencies.[/bold red]")

def cmd_gh_runs(args):
    projects = get_projects()
    for p in projects:
        print(f"\n[bold blue]=== GitHub Runs: {p.name} ===[/bold blue]")
        try:
            result = subprocess.run(
                ["gh", "run", "list", "--limit", "3"],
                cwd=p,
                capture_output=True,
                text=True
            )
            if result.returncode == 0:
                if not result.stdout.strip():
                    print("  No runs found.")
                else:
                    for line in result.stdout.splitlines():
                        if "✓" in line: print(f"  [green]{line}[/green]")
                        elif "X" in line or "fail" in line.lower(): print(f"  [red]{line}[/red]")
                        elif "*" in line: print(f"  [yellow]{line}[/yellow]")
                        else: print(f"  {line}")
            else:
                print(f"  [dim]GH CLI Error or no workflows configured.[/dim]")
        except Exception as e:
            print(f"  Error: {e}")

def cmd_convert(args):
    from media_converter import convert_audio, convert_video, convert_image, check_ffmpeg, check_armake
    
    for f in args.files:
        if not os.path.exists(f):
            print(f"⚠️ File not found: {f}")
            continue
        ext = os.path.splitext(f)[1].lower()
        if ext in [".wav", ".mp3", ".m4a", ".flac"]:
            if check_ffmpeg(): convert_audio(f)
            else: print(f"❌ Error: ffmpeg required for {f}")
        elif ext in [".mp4", ".mkv", ".mov", ".avi"]:
            if check_ffmpeg(): convert_video(f)
            else: print(f"❌ Error: ffmpeg required for {f}")
        elif ext in [".png", ".jpg", ".jpeg"]:
            if check_armake(): convert_image(f)
            else: print(f"❌ Error: armake required for {f}")
        else:
            print(f"❓ Unknown format for {f}")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Workspace Management Tool")
    subparsers = parser.add_subparsers(dest="command", help="Management commands")

    subparsers.add_parser("dashboard", help="Visual workspace overview")
    subparsers.add_parser("status", help="Show status of all projects")
    subparsers.add_parser("sync", help="Run mod manager sync on all projects")
    subparsers.add_parser("build", help="Run HEMTT build on all projects")
    subparsers.add_parser("release", help="Run release script")
    subparsers.add_parser("test", help="Run tests")
    subparsers.add_parser("clean", help="Clean build artifacts")
    subparsers.add_parser("cache", help="Show disk usage")
    subparsers.add_parser("publish", help="Upload to Steam")
    subparsers.add_parser("validate", help="Run validators")
    subparsers.add_parser("audit-build", help="Audit builds")
    subparsers.add_parser("update", help="Push latest tools")
    subparsers.add_parser("workshop-tags", help="List Workshop Tags")
    subparsers.add_parser("gh-runs", help="GitHub Action status")
    subparsers.add_parser("audit-deps", help="Audit project requiredAddons dependencies")

    convert_parser = subparsers.add_parser("convert", help="Convert media (.ogg/.ogv/.paa)")
    convert_parser.add_argument("files", nargs="+", help="Files to convert")

    args = parser.parse_args()

    if args.command == "dashboard": cmd_dashboard(args)
    elif args.command == "status": cmd_status(args)
    elif args.command == "sync": cmd_sync(args)
    elif args.command == "build": cmd_build(args)
    elif args.command == "release": cmd_release(args)
    elif args.command == "test": cmd_test(args)
    elif args.command == "clean": cmd_clean(args)
    elif args.command == "cache": cmd_cache(args)
    elif args.command == "publish": cmd_publish(args)
    elif args.command == "validate": cmd_validate(args)
    elif args.command == "audit-build": cmd_audit_build(args)
    elif args.command == "update": cmd_update(args)
    elif args.command == "workshop-tags": cmd_workshop_tags(args)
    elif args.command == "gh-runs": cmd_gh_runs(args)
    elif args.command == "audit-deps": cmd_audit_deps(args)
    elif args.command == "convert": cmd_convert(args)
    else: parser.print_help()

if __name__ == "__main__":
    main()
