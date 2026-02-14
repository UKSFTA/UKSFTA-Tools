#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
import re
from pathlib import Path

# --- CONFIGURATION ---
TOOLS_ROOT = Path(__file__).parent.parent
DEBINARIZER = TOOLS_ROOT / "bin" / "linux-x64" / "debinarizer"

# Performance Thresholds
POLY_LIMIT_LOD0 = 25000  # Vertices in first LOD
TEXTURE_LIMIT_MB = 15.0  # Single texture size in MB

def get_p3d_vertices(p3d_path):
    """Extracts vertex counts for each LOD."""
    if not DEBINARIZER.exists(): return {}
    
    lods = {}
    cmd = [str(DEBINARIZER), str(p3d_path), "-info"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
        # Parse: "    - 1.0: 1234 pts, 2 textures"
        matches = re.finditer(r"-\s+([\d\.E\+\-]+):\s+(\d+)\s+pts", res.stdout)
        for m in matches:
            lods[m.group(1)] = int(m.group(2))
    except: pass
    return lods

def report_weight(project_path):
    project_path = Path(project_path)
    print(f"\n📊 [Performance Analytics] {project_path.name}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

    heavy_models = []
    heavy_textures = []
    
    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            path = Path(root) / f
            
            # 1. Analyze Models
            if f.endswith(".p3d"):
                lods = get_p3d_vertices(path)
                # Check Resolution 0.0 or 1.0 (typically visual LOD 0)
                lod0_verts = lods.get("0.0", lods.get("1.0", 0))
                if lod0_verts > POLY_LIMIT_LOD0:
                    heavy_models.append((f, lod0_verts))
            
            # 2. Analyze Textures
            if f.endswith(".paa"):
                size_mb = os.path.getsize(path) / (1024 * 1024)
                if size_mb > TEXTURE_LIMIT_MB:
                    heavy_textures.append((f, size_mb))

    # Output Report
    if not heavy_models and not heavy_textures:
        print("  ✅ PASS: No significant performance bottlenecks detected.")
        return

    if heavy_models:
        print(f"\n  [⚠️  High-Poly Warning] (> {POLY_LIMIT_LOD0} verts)")
        for m, v in sorted(heavy_models, key=lambda x: x[1], reverse=True):
            print(f"    - {m:<30} | {v:>6} Vertices")

    if heavy_textures:
        print(f"\n  [⚠️  Large Texture Warning] (> {TEXTURE_LIMIT_MB} MB)")
        for t, s in sorted(heavy_textures, key=lambda x: x[1], reverse=True):
            print(f"    - {t:<30} | {s:>6.2f} MB")

    print("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: weight_reporter.py <project_path>")
        sys.exit(1)
    report_weight(sys.argv[1])
