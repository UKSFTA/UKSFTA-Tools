#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import time
import subprocess
import sys
from pathlib import Path

# --- CONFIGURATION ---
WATCH_EXTENSIONS = {".paa", ".p3d"}
POLL_INTERVAL = 5 # seconds

def get_modified_files(root_dir, last_check_time):
    modified = []
    for root, _, files in os.walk(root_dir):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            path = Path(root) / f
            if path.suffix.lower() in WATCH_EXTENSIONS:
                try:
                    mtime = path.stat().st_mtime
                    if mtime > last_check_time: modified.append(path)
                except: pass
    return modified

def agent_loop(target_dir):
    print(f"
🤖 [THE SENTINEL] Active Watchdog v3.0")
    print(f"[*] Watching: {target_dir}")
    print(f"[*] Polling every {POLL_INTERVAL} seconds...")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    last_check = time.time()
    try:
        while True:
            time.sleep(POLL_INTERVAL)
            new_changes = get_modified_files(target_dir, last_check)
            last_check = time.time()
            if not new_changes: continue
            print(f"
👀 Detected {len(new_changes)} modified assets.")
            for f in new_changes:
                print(f"  👉 Analyzing: {f.name}")
                if f.suffix.lower() == ".paa": subprocess.run([sys.executable, "tools/asset_optimizer.py", str(f)])
                elif f.suffix.lower() == ".p3d": subprocess.run([sys.executable, "tools/rebin_guard.py", str(f)])
    except KeyboardInterrupt: print("
🛑 Sentinel Deactivated.")

if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else "."
    agent_loop(target)
