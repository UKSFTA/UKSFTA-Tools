#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import requests
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

def download_tile(url, dest):
    """Downloads a single map tile."""
    if dest.exists(): return
    
    try:
        response = requests.get(url, timeout=10)
        if response.status_code == 200:
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(response.content)
            return True
    except: pass
    return False

def mirror_theatre(theatre_name, web_root, max_zoom=5):
    """
    Downloads all tiles for a theatre from community archives into local static storage.
    """
    print(f"📡 [Intel Mirror] Archiving theatre: {theatre_name}")
    
    # Try different community URL patterns
    base_urls = [
        f"https://tiles.plan-ops.fr/tiles/{theatre_name}",
        f"https://tiles.plan-ops.fr/{theatre_name}",
        f"https://jetelain.github.io/Arma3Map/tiles/{theatre_name}"
    ]
    
    local_dir = Path(web_root) / "static" / "theatre" / theatre_name
    
    with ThreadPoolExecutor(max_workers=30) as executor:
        for base_url in base_urls:
            print(f"  └─ Probing Source: {base_url}")
            futures = []
            for z in range(max_zoom + 1):
                grid_size = 2 ** z
                for x in range(grid_size):
                    for y in range(grid_size):
                        url = f"{base_url}/{z}/{x}/{y}.png"
                        dest = local_dir / str(z) / str(x) / f"{y}.png"
                        futures.append(executor.submit(download_tile, url, dest))
            
            results = [f.result() for f in futures]
            success_count = sum(1 for r in results if r)
            if success_count > 0:
                print(f"✅ Success! {success_count} tiles archived from source.")
                break
            else:
                print(f"  ❌ Source yielded no data.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: intel_mirror.py <theatre_name>")
        sys.exit(1)
        
    # Detect if we are running in a project root or tools folder
    web_dir = Path("web")
    if not web_dir.exists() and Path("../web").exists():
        web_dir = Path("../web")
        
    mirror_theatre(sys.argv[1], web_dir)
