#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
import re
import json
from pathlib import Path

# --- CONFIGURATION ---
TOOLS_ROOT = Path(__file__).parent.parent
DEBINARIZER = TOOLS_ROOT / "bin" / "linux-x64" / "debinarizer"

def manage_proxies(p3d_path, action, proxy_name=None, position=None):
    """
    Manages proxies within a P3D file.
    Currently leverages the debinarizer binary for extraction and planned injection.
    """
    if not DEBINARIZER.exists():
        print("❌ Error: Forensic binary missing.")
        return

    if action == "list":
        cmd = [str(DEBINARIZER), str(p3d_path), "-info"]
        try:
            res = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
            # Find [Proxies] section
            pattern = re.escape("[Proxies]") + r"(.*?)(?=
\s*\[|$)"
            m = re.search(pattern, res.stdout, re.DOTALL)
            if m:
                print(f"
[*] Proxies in {os.path.basename(p3d_path)}:")
                lines = m.group(1).strip().splitlines()
                for l in lines:
                    if l.strip().startswith("-"):
                        print(f"  {l.strip()}")
            else:
                print(f"
[*] No proxies found in {os.path.basename(p3d_path)}.")
        except Exception as e:
            print(f"❌ Error listing proxies: {e}")

    elif action == "sanitize":
        print(f"[*] Sanitizing orphaned proxies in {os.path.basename(p3d_path)}...")
        # To be implemented: Full P3D rewrite logic
        print("  ⚠️  Sanitization requires full P3D write support (Planned).")

    elif action == "inject":
        print(f"[*] Injecting proxy '{proxy_name}' at {position} in {os.path.basename(p3d_path)}...")
        # To be implemented: Full P3D rewrite logic
        print("  ⚠️  Injection requires full P3D write support (Planned).")

def main():
    import argparse
    parser = argparse.ArgumentParser(description="UKSFTA Proxy Manager")
    parser.add_argument("file", help="Path to P3D file")
    parser.add_argument("action", choices=["list", "sanitize", "inject"], help="Action to perform")
    parser.add_argument("--proxy", help="Proxy model name (for injection)")
    parser.add_argument("--pos", help="Position [x,y,z] (for injection)")
    
    args = parser.parse_args()
    
    if args.action == "inject" and not (args.proxy and args.pos):
        print("❌ Error: --proxy and --pos required for injection.")
        sys.exit(1)
        
    manage_proxies(args.file, args.action, args.proxy, args.pos)

if __name__ == "__main__":
    main()
