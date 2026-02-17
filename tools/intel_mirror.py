#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import requests
import json
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

def download_tile(url, dest):
    """Downloads a single map tile."""
    if dest.exists(): return
    try:
        response = requests.get(url, timeout=5)
        if response.status_code == 200:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(response.content)
            return True
    except: pass
    return False

def mirror_theatre(theatre_name, web_root, max_zoom=5):
    """Downloads all tiles for a specific theatre."""
    print(f"📡 [Intel Mirror] Archiving: {theatre_name}")
    base_urls = [
        f"https://tiles.plan-ops.fr/tiles/{theatre_name}",
        f"https://jetelain.github.io/Arma3Map/tiles/{theatre_name}"
    ]
    local_dir = Path(web_root) / "static" / "theatre" / theatre_name
    
    with ThreadPoolExecutor(max_workers=20) as executor:
        for base_url in base_urls:
            futures = []
            for z in range(max_zoom + 1):
                grid_size = 2 ** z
                for x in range(grid_size):
                    for y in range(grid_size):
                        url = f"{base_url}/{z}/{x}/{y}.png"
                        dest = local_dir / str(z) / str(x) / f"{y}.png"
                        futures.append(executor.submit(download_tile, url, dest))
            
            if any(f.result() for f in futures):
                print(f"  ✅ Source found for {theatre_name}")
                return True
    return False

def mirror_all(web_root):
    """Iterates through all known maps and archives them."""
    registry_path = Path(web_root) / "static" / "community" / "maps" / "all.json"
    if not registry_path.exists():
        print("❌ Error: Map registry (all.json) not found. Run intel_sync first.")
        return

    with open(registry_path, 'r') as f:
        maps = json.load(f)
    
    total = len(maps)
    print(f"🌍 [Mass Archive] Preparing to mirror {total} terrains...")
    
    for i, world_name in enumerate(maps.keys()):
        print(f"[{i+1}/{total}] Processing {world_name}...")
        mirror_theatre(world_name.lower(), web_root)

if __name__ == "__main__":
    web_dir = Path("web")
    if not web_dir.exists() and Path("../web").exists():
        web_dir = Path("../web")

    if len(sys.argv) < 2:
        print("Usage: intel_mirror.py <theatre_name|--all>")
        sys.exit(1)
        
    if sys.argv[1] == "--all":
        mirror_all(web_dir)
    else:
        mirror_theatre(sys.argv[1], web_dir)
