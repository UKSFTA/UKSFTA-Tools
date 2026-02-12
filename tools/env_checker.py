#!/usr/bin/env python3
import os
import sys
import shutil
import subprocess
from pathlib import Path

# UKSFTA Environment Checker
# Verifies all required dependencies are installed and configured.

def check_command(cmd, name):
    path = shutil.which(cmd)
    if path:
        try:
            # Try to get version if possible
            ver = "Installed"
            if cmd == "hemtt":
                res = subprocess.run(["hemtt", "--version"], capture_output=True, text=True)
                ver = res.stdout.strip()
            elif cmd == "python3":
                ver = sys.version.split()[0]
            print(f"  [bold green]✅ {name:<15}[/] : {ver}")
            return True
        except:
            print(f"  [bold green]✅ {name:<15}[/] : Installed")
            return True
    else:
        print(f"  [bold red]❌ {name:<15}[/] : NOT FOUND")
        return False

def main():
    print("
🔍 [bold blue]UKSFTA Development Environment Audit[/bold blue]")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    
    critical = []
    critical.append(check_command("python3", "Python 3"))
    critical.append(check_command("git", "Git"))
    critical.append(check_command("hemtt", "HEMTT"))
    critical.append(check_command("steamcmd", "SteamCMD"))
    critical.append(check_command("zip", "Zip"))
    
    print("
[Assurance & Security]")
    check_command("gh", "GitHub CLI")
    check_command("gpg", "GPG (Signing)")
    
    # Check for Rich
    try:
        from rich import print as rprint
        print(f"  [bold green]✅ {'Rich (UI)':<15}[/] : Installed")
    except ImportError:
        print(f"  [bold yellow]⚠️  {'Rich (UI)':<15}[/] : Not found (install via pip)")

    if all(critical):
        print("
[bold green]✨ Environment is production-ready![/bold green]
")
    else:
        print("
[bold red]⚠️  Critical dependencies are missing. Please check docs/SETUP.md[/bold red]
")

if __name__ == "__main__":
    try:
        from rich import print
    except ImportError:
        pass
    main()
