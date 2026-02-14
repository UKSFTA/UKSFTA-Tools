#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
import shutil
import tempfile
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

def optimize_texture(paa_path, apply=False):
    """
    Downscales a PAA texture if it exceeds thresholds.
    Workflow: PAA -> PNG -> Resize -> PAA
    """
    path = Path(paa_path)
    suffix = next((s for s in LIMITS.keys() if path.stem.lower().endswith(s)), "default")
    limit = LIMITS[suffix]

    # 1. Get current dimensions using hemtt
    try:
        res = subprocess.run(["hemtt", "utils", "paa", "inspect", str(path)], capture_output=True, text=True)
        # Parse: "Dimensions: 4096x4096"
        match = re.search(r"Dimensions: (\d+)x(\d+)", res.stdout)
        if not match: return False
        width = int(match.group(1))
    except:
        # Fallback to magick identify if hemtt fails
        try:
            res = subprocess.run(["magick", "identify", "-format", "%w", str(path)], capture_output=True, text=True)
            width = int(res.stdout.strip())
        except: return False

    if width > limit:
        print(f"  ⚡ Detected: {path.name} ({width}px -> {limit}px)")
        if not apply: return True # Just reporting
        
        with tempfile.TemporaryDirectory(prefix="uksfta_opt_") as tmpdir:
            tmp_png = Path(tmpdir) / "temp.png"
            try:
                # A. Convert PAA -> PNG
                subprocess.run(["hemtt", "utils", "paa", "convert", str(path), str(tmp_png)], 
                             capture_output=True, check=True)
                
                # B. Resize PNG
                subprocess.run(["magick", str(tmp_png), "-resize", f"{limit}x{limit}", str(tmp_png)], 
                             check=True)
                
                # C. Convert PNG -> PAA (Overwrite original)
                subprocess.run(["hemtt", "utils", "paa", "convert", str(tmp_png), str(path)], 
                             capture_output=True, check=True)
                
                print(f"    ✅ Successfully optimized and overwrote: {path.name}")
                return True
            except Exception as e:
                print(f"    ❌ Optimization failed for {path.name}: {e}")
                return False
    return False

def run_optimizer(project_path, apply=False):
    project_path = Path(project_path)
    print(f"\n🚀 [Asset Optimizer] Scanning: {project_path.name}")
    if apply: print("🔥 APPLY MODE ENABLED: Overwriting textures with optimized versions.")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    
    count = 0
    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            if f.lower().endswith(".paa"):
                if optimize_texture(os.path.join(root, f), apply):
                    count += 1
                    
    print(f"\n✨ Optimization cycle complete. Processed {count} assets.")

if __name__ == "__main__":
    import argparse
    import re
    parser = argparse.ArgumentParser(description="UKSFTA Active Asset Optimizer")
    parser.add_argument("path", help="Project or mod directory to scan")
    parser.add_argument("--apply", action="store_true", help="Apply optimization (overwrite files)")
    
    args = parser.parse_args()
    run_optimizer(args.path, args.apply)
