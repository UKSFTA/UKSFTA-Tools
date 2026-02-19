#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import shutil
from pathlib import Path

def sync_community_intel(project_root):
    """
    Clones community map definitions into a dedicated data archive.
    """
    repo_url = "https://github.com/jetelain/Arma3Map.git"
    temp_dir = Path(".community_intel_temp")
    # New Standard: Dedicated archive outside of Hugo's build path
    archive_dir = Path(project_root) / "data_archive" / "community"
    
    print(f"📡 [Intel Sync] Fetching Community Archive to: {archive_dir}")
    
    if temp_dir.exists(): shutil.rmtree(temp_dir)
    
    try:
        subprocess.run(["git", "clone", "--depth", "1", repo_url, str(temp_dir)], check=True)
        archive_dir.mkdir(parents=True, exist_ok=True)
        
        print(f"  └─ Archiving intelligence metadata...")
        if (archive_dir / "maps").exists(): shutil.rmtree(archive_dir / "maps")
        shutil.copytree(temp_dir / "maps", archive_dir / "maps")
        
        if (temp_dir / "icons").exists():
            if (archive_dir / "icons").exists(): shutil.rmtree(archive_dir / "icons")
            shutil.copytree(temp_dir / "icons", archive_dir / "icons")

        # Sync ONLY the master registry back to Hugo static for discovery
        print(f"  └─ Updating Hugo discovery registry...")
        hugo_registry = Path(project_root) / "web" / "static" / "community" / "maps"
        hugo_registry.mkdir(parents=True, exist_ok=True)
        shutil.copy2(archive_dir / "maps" / "all.json", hugo_registry / "all.json")

        print(f"✅ Sync Complete. Build speed preserved.")
        
    except Exception as e:
        print(f"❌ Sync Failed: {e}")
    finally:
        if temp_dir.exists(): shutil.rmtree(temp_dir)

if __name__ == "__main__":
    # Detect project root
    root = Path(".")
    if not (root / "web").exists() and Path("..").exists(): root = Path("..")
    sync_community_intel(root)
