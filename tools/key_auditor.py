#!/usr/bin/env python3
import os
import sys
from pathlib import Path

# UKSFTA Signature Security Auditor
# Verifies presence of official unit keys and warns about rogue keys.

def audit_project_keys(project_path):
    root = Path(project_path)
    print(f"🔑 Auditing Keys for: {root.name}")
    
    keys_dir = root / "keys"
    if not keys_dir.exists():
        print("  i No keys directory found.")
        return

    # Official Unit Keys (Case-insensitive)
    OFFICIAL_KEYS = ["uksfta.bikey", "uksfta_v1.bikey"]
    
    rogue_keys = []
    found_official = False
    
    for key in keys_dir.glob("*.bikey"):
        if key.name.lower() in OFFICIAL_KEYS:
            found_official = True
        else:
            rogue_keys.append(key.name)

    if not found_official:
        print("  [bold red]❌ CRITICAL[/] : Missing official UKSFTA public key!")
    else:
        print("  [bold green]✅ OK[/] : Official UKSFTA key found.")

    if rogue_keys:
        print(f"  [bold yellow]⚠️  WARNING[/] : Rogue public keys detected (remove these!):")
        for r in rogue_keys:
            print(f"     - {r}")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: audit_keys.py <project_path>")
        sys.exit(1)
    
    try:
        from rich import print
    except ImportError: pass
    
    audit_project_keys(sys.argv[1])
