#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
from pathlib import Path

# --- CONFIGURATION ---
# Max resolution for different texture types
LIMITS = {
    "default": 2048,
    "_nohq": 2048, # Normal maps
    "_smdi": 1024, # Specular
    "_ca": 1024,   # UI / Icons
    "_co": 2048    # Color
}

def optimize_texture(paa_path, dry_run=False):
    """
    Downscales a PAA texture if it exceeds thresholds.
    """
    path = Path(paa_path)
    suffix = next((s for s in LIMITS.keys() if path.stem.lower().endswith(s)), "default")
    limit = LIMITS[suffix]

    # Get current dimensions
    try:
        res = subprocess.run(["magick", "identify", "-format", "%w", str(path)], capture_output=True, text=True)
        width_str = res.stdout.strip()
        if not width_str: return False
        width = int(width_str)
    except: return False

    if width > limit:
        print(f"  ⚡ Detected: {path.name} ({width}px -> {limit}px)")
        if dry_run: return True
        
        tmp_png = path.with_suffix(".png")
        try:
            # 1. Convert to PNG
            subprocess.run(["magick", str(path), str(tmp_png)], check=True)
            # 2. Resize
            subprocess.run(["magick", str(tmp_png), "-resize", f"{limit}x{limit}", str(tmp_png)], check=True)
            print(f"    ✅ Resized {tmp_png.name}. Re-export to PAA required.")
            return True
        except Exception as e:
            print(f"    ❌ Optimization failed: {e}")
            return False
    return False

def run_optimizer(project_path, dry_run=False):
    project_path = Path(project_path)
    print(f"\n🚀 [Asset Optimizer] Scanning: {project_path.name}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    
    count = 0
    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            if f.lower().endswith(".paa"):
                if optimize_texture(os.path.join(root, f), dry_run):
                    count += 1
                    
    print(f"\n✨ Optimization scan complete. Identified {count} assets.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: asset_optimizer.py <project_path> [--apply]")
        sys.exit(1)
    
    dry = "--apply" not in sys.argv
    run_optimizer(sys.argv[1], dry)
