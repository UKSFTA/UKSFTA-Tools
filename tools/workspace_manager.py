#!/usr/bin/env python3

import os
import sys
import subprocess
import argparse
import re
from pathlib import Path

try:
    from rich.console import Console
    from rich.table import Table
    from rich.panel import Panel
    from rich import box
    HAS_RICH = True
except ImportError:
    HAS_RICH = False

# UKSFTA Workspace Manager
# Centralized control for multi-project mod environments

def get_projects(root_dir=".."):
    projects = []
    root = Path(root_dir).resolve()
    for item in root.iterdir():
        if item.is_dir() and item.name.startswith("UKSFTA-") and item.name not in ["UKSFTA-Tools"]:
            if (item / ".hemtt" / "project.toml").exists():
                projects.append(item)
    return sorted(projects)

def get_version(project_path):
    version_file = project_path / "addons" / "main" / "script_version.hpp"
    if not version_file.exists(): return "?.?.?"
    content = version_file.read_text()
    major = re.search(r"#define\s+MAJOR\s+(\d+)", content)
    minor = re.search(r"#define\s+MINOR\s+(\d+)", content)
    patch = re.search(r"#define\s+PATCHLVL\s+(\d+)", content)
    if major and minor and patch:
        return f"{major.group(1)}.{minor.group(1)}.{patch.group(1)}"
    return "?.?.?"

def cmd_dashboard(args):
    projects = get_projects()
    
    if not HAS_RICH:
        print(f"{'Project':<25} | {'Version':<8} | {'Sync':<5} | {'Build':<5} | {'Health':<10}")
        print("-" * 65)
        for p in projects:
            version = get_version(p)
            sync = "✔" if (p / "mods.lock").exists() else "!"
            build = "✔" if (p / ".hemttout" / "release").exists() else "✖"
            health = "Stable" if (p / "mods.lock").exists() else "Unsynced"
            print(f"{p.name:<25} | {version:<8} | {sync:<5} | {build:<5} | {health:<10}")
        return

    console = Console()
    table = Table(title="UKSFTA Mod Workspace Dashboard", box=box.ROUNDED, header_style="bold cyan")
    table.add_column("Project", style="bold white")
    table.add_column("Version", justify="center")
    table.add_column("Workshop ID", justify="center", style="dim")
    table.add_column("Sync", justify="center")
    table.add_column("Build", justify="center")
    table.add_column("Health", justify="center")

    for p in projects:
        # Version
        version = get_version(p)
        
        # Workshop ID
        ws_id = "None"
        toml = p / ".hemtt" / "project.toml"
        if toml.exists():
            match = re.search(r'workshop_id = "(.*?)"', toml.read_text())
            if match: ws_id = match.group(1)
        
        # Sync Status
        sync_icon = "[green]✔[/green]" if (p / "mods.lock").exists() else "[yellow]![/yellow]"
        
        # Build Status
        build_icon = "[red]✖[/red]"
        release_dir = p / ".hemttout" / "release"
        if release_dir.exists():
            build_icon = "[green]✔[/green]"
            
        # Overall Health (Quick check)
        health = "[green]Stable[/green]"
        if ws_id == "0" or ws_id == "None":
            health = "[yellow]No ID[/yellow]"
        if not (p / "mods.lock").exists():
            health = "[red]Unsynced[/red]"

        table.add_row(
            p.name,
            version,
            ws_id,
            sync_icon,
            build_icon,
            health
        )

    console.print(Panel(table, expand=False, border_style="blue"))

def run_in_project(project_path, cmd):
    print(f"\n>>> Project: {project_path.name}")
    try:
        result = subprocess.run(cmd, cwd=project_path, check=True, text=True, capture_output=True)
        print(result.stdout)
        return True
    except subprocess.CalledProcessError as e:
        print(f"Error in {project_path.name}:")
        print(e.stdout)
        print(e.stderr)
        return False

def cmd_status(args):
    projects = get_projects()
    print(f"{'Project':<25} | {'HEMTT':<6} | {'Sync':<10} | {'Artifacts (K/S)':<15}")
    print("-" * 65)
    for p in projects:
        has_hemtt = "Yes" if (p / ".hemtt" / "project.toml").exists() else "No"
        sync_state = "Synced" if (p / "mods.lock").exists() else "Pending"
        
        keys = len(list((p / "keys").glob("*.bikey"))) if (p / "keys").exists() else 0
        signs = len(list((p / "addons").glob("*.bisign"))) if (p / "addons").exists() else 0
        
        print(f"{p.name:<25} | {has_hemtt:<6} | {sync_state:<10} | {keys}/{signs}")

def cmd_sync(args):
    projects = get_projects()
    for p in projects:
        if (p / "tools" / "manage_mods.py").exists():
            print(f"--- Syncing {p.name} ---")
            run_in_project(p, [sys.executable, "tools/manage_mods.py"])
        else:
            print(f"Skipping {p.name}: No Mod Manager found.")

def cmd_build(args):
    projects = get_projects()
    for p in projects:
        if (p / "build.sh").exists():
            print(f"--- Building {p.name} ---")
            run_in_project(p, ["bash", "build.sh", "build"])
        else:
            print(f"Skipping {p.name}: No build.sh found.")

def cmd_release(args):
    projects = get_projects()
    for p in projects:
        if (p / "release.sh").exists():
            print(f"--- Packaging Release: {p.name} ---")
            run_in_project(p, ["bash", "release.sh"])
        else:
            print(f"Skipping {p.name}: No release.sh found.")

def cmd_test(args):
    console = Console() if HAS_RICH else None
    projects = get_projects()
    tools_dir = Path(__file__).parent.resolve()
    
    # 1. Run Python Unit Tests for Tools
    print("\n[bold cyan]Step 1: Running Python Tool Tests (pytest)[/bold cyan]" if HAS_RICH else "\nStep 1: Running Python Tool Tests (pytest)")
    subprocess.run(["pytest", str(tools_dir / "tests")], cwd=tools_dir.parent)

    # 2. Run Workspace-wide Checks
    for p in projects:
        print(f"\n[bold blue]=== Testing Project: {p.name} ===[/bold blue]" if HAS_RICH else f"\n=== Testing Project: {p.name} ===")
        
        # A. HEMTT Check
        print("  - Running HEMTT Check...")
        subprocess.run(["hemtt", "check"], cwd=p)

        # B. SQFLint
        print("  - Running SQFLint...")
        addons_dir = p / "addons"
        if addons_dir.exists():
            # sqflint -d <dir> is the correct way to lint a directory
            subprocess.run(["sqflint", "-d", str(addons_dir.resolve())], cwd=p)
        else:
            print("    (No addons directory found)")

        # C. UKSFTA Custom Validators
        print("  - Running UKSFTA Validators...")
        validators = ["config_style_checker.py", "stringtable_validator.py", "return_checker.py"]
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
            shutil.rmtree(out_dir)
        else:
            print(f"Skipping {p.name}: No .hemttout found.")

def cmd_cache(args):
    projects = get_projects()
    total_bytes = 0
    print(f"{'Project':<25} | {'Cache Size':<15}")
    print("-" * 45)
    for p in projects:
        out_dir = p / ".hemttout"
        size = 0
        if out_dir.exists():
            for f in out_dir.rglob("*"):
                if f.is_file():
                    size += f.stat().st_size
        
        total_bytes += size
        size_mb = size / (1024 * 1024)
        print(f"{p.name:<25} | {size_mb:>10.2f} MB")
    
    total_gb = total_bytes / (1024 * 1024 * 1024)
    print("-" * 45)
    print(f"{'TOTAL':<25} | {total_gb:>10.2f} GB")

def cmd_publish(args):
    projects = get_projects()
    publishable = []
    
    for p in projects:
        ws_id = "None"
        toml = p / ".hemtt" / "project.toml"
        if toml.exists():
            match = re.search(r'workshop_id = "(.*?)"', toml.read_text())
            if match: ws_id = match.group(1)
        
        if ws_id not in ["0", "None", "INSERT_ID_HERE", ""]:
            publishable.append((p, ws_id))
    
    if not publishable:
        print("No projects found with valid Workshop IDs.")
        return

    if HAS_RICH:
        mode_label = "[bold red]PRODUCTION UPLOAD[/bold red]" if not args.dry_run else "[bold green]DRY-RUN SIMULATION[/bold green]"
        print(f"\nTarget Projects for {mode_label}:")
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
    tools_dir = Path(__file__).parent.resolve()
    validators = [
        "config_style_checker.py",
        "sqf_validator.py",
        "stringtable_validator.py",
        "return_checker.py",
        "search_unused_privates.py"
    ]
    
    for p in projects:
        print(f"\n=== Validating {p.name} ===")
        for val in validators:
            val_path = tools_dir / val
            if val_path.exists():
                subprocess.run([sys.executable, str(val_path)], cwd=p)

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

def cmd_convert(args):
    from media_converter import convert_audio, convert_video, check_ffmpeg
    if not check_ffmpeg():
        print("❌ Error: ffmpeg not found. Please install it (sudo pacman -S ffmpeg)")
        return
    
    for f in args.files:
        if not os.path.exists(f):
            print(f"⚠️ File not found: {f}")
            continue
        ext = os.path.splitext(f)[1].lower()
        if ext in [".wav", ".mp3", ".m4a", ".flac"]:
            convert_audio(f)
        elif ext in [".mp4", ".mkv", ".mov", ".avi"]:
            convert_video(f)
        else:
            print(f"❓ Unknown format for {f}")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Workspace Management Tool")
    subparsers = parser.add_subparsers(dest="command", help="Management commands")

    subparsers.add_parser("dashboard", help="Visual workspace overview")
    subparsers.add_parser("status", help="Show status of all projects")
    subparsers.add_parser("sync", help="Run mod manager sync on all projects")
    subparsers.add_parser("build", help="Run HEMTT build on all projects")
    subparsers.add_parser("release", help="Run UKSFTA release script (ZIP packaging) on all projects")
    subparsers.add_parser("test", help="Run full suite of tests (pytest, sqflint, hemtt check)")
    subparsers.add_parser("clean", help="Clean all build artifacts (.hemttout)")
    subparsers.add_parser("cache", help="Show disk usage of .hemttout across workspace")
    
    publish_parser = subparsers.add_parser("publish", help="Upload all projects with valid IDs to Steam Workshop")
    publish_parser.add_argument("--dry-run", action="store_true", help="Simulate upload and validate without talking to Steam")
    
    subparsers.add_parser("validate", help="Run all validators on all projects")
    subparsers.add_parser("audit-build", help="Run integrity check on built artifacts (.hemttout)")
    subparsers.add_parser("update", help="Push latest tools/setup to all projects")
    
    convert_parser = subparsers.add_parser("convert", help="Convert media to Arma-optimized formats (.ogg/.ogv)")
    convert_parser.add_argument("files", nargs="+", help="Files to convert")

    args = parser.parse_args()

    if args.command == "dashboard":
        cmd_dashboard(args)
    elif args.command == "status":
        cmd_status(args)
    elif args.command == "sync":
        cmd_sync(args)
    elif args.command == "build":
        cmd_build(args)
    elif args.command == "release":
        cmd_release(args)
    elif args.command == "test":
        cmd_test(args)
    elif args.command == "clean":
        cmd_clean(args)
    elif args.command == "cache":
        cmd_cache(args)
    elif args.command == "publish":
        cmd_publish(args)
    elif args.command == "validate":
        cmd_validate(args)
    elif args.command == "audit-build":
        cmd_audit_build(args)
    elif args.command == "update":
        cmd_update(args)
    elif args.command == "convert":
        cmd_convert(args)
    else:
        parser.print_help()

if __name__ == "__main__":
    main()
