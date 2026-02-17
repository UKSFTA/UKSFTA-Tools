#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import shutil
from pathlib import Path

def sync_community_intel(web_root):
    """
    Clones/Updates the community map definitions from jetelain/Arma3Map.
    """
    repo_url = "https://github.com/jetelain/Arma3Map.git"
    temp_dir = Path(".community_intel_temp")
    target_dir = Path(web_root) / "static" / "community"
    
    print(f"📡 [Intel Sync] Fetching Community Terrain Archive...")
    
    if temp_dir.exists(): shutil.rmtree(temp_dir)
    
    try:
        # Shallow clone to save space and time
        subprocess.run(["git", "clone", "--depth", "1", repo_url, str(temp_dir)], check=True)
        
        target_dir.mkdir(parents=True, exist_ok=True)
        
        print(f"  └─ Archiving map definitions...")
        if (target_dir / "maps").exists(): shutil.rmtree(target_dir / "maps")
        shutil.copytree(temp_dir / "maps", target_dir / "maps")
        
        if (temp_dir / "icons").exists():
            if (target_dir / "icons").exists(): shutil.rmtree(target_dir / "icons")
            shutil.copytree(temp_dir / "icons", target_dir / "icons")

        print(f"✅ Sync Complete. Terrains archived locally.")
        
    except Exception as e:
        print(f"❌ Sync Failed: {e}")
    finally:
        if temp_dir.exists(): shutil.rmtree(temp_dir)

if __name__ == "__main__":
    # We detect if we are running in a project root or tools folder
    web_dir = Path("web")
    if not web_dir.exists() and Path("../web").exists():
        web_dir = Path("../web")
        
    sync_community_intel(web_dir)
