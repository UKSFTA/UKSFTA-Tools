#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import shutil
from pathlib import Path

def setup_project():
    # Detect Project Root (Parent of .uksf_tools or current dir)
    if Path(".uksf_tools").exists():
        project_root = Path(".")
        tools_src = Path(".uksf_tools")
    else:
        project_root = Path("..")
        tools_src = Path(".")

    print(f"Setting up UKSFTA Tools in: {project_root.resolve()}")

    # 1. Symlink tools/
    tools_link = project_root / "tools"
    if tools_link.exists():
        if tools_link.is_symlink(): os.unlink(tools_link)
        else: shutil.rmtree(tools_link)
    
    try:
        os.symlink(".uksf_tools/tools", tools_link)
        print("  ✅ Symlinked: tools/ directory -> .uksf_tools/tools")
    except:
        shutil.copytree(tools_src / "tools", tools_link)
        print("  ✅ Copied: tools/ directory (Symlink failed)")

    # 2. Refresh HEMTT Hooks & Scripts (Force Overwrite)
    # Source is 'hemtt/', Destination is '.hemtt/'
    for folder in ["hooks", "scripts"]:
        src = tools_src / "hemtt" / folder
        dest = project_root / ".hemtt" / folder
        if src.exists():
            if dest.exists(): shutil.rmtree(dest)
            # Ensure parent exists
            dest.parent.mkdir(exist_ok=True)
            shutil.copytree(src, dest)
            print(f"  ✅ Refreshed: .hemtt/{folder}")

    # 3. Refresh GitHub Workflows
    github_src = tools_src / ".github" / "workflows"
    github_dest = project_root / ".github" / "workflows"
    if github_src.exists():
        project_root.joinpath(".github").mkdir(exist_ok=True)
        if github_dest.exists(): shutil.rmtree(github_dest)
        shutil.copytree(github_src, github_dest)
        print(f"  ✅ Refreshed: .github/workflows")

    # 4. Update Entry Points (Critical: build.sh, release.sh)
    for script in ["build.sh", "release.sh", "bootstrap.sh", "install_mikero.sh"]:
        src = tools_src / script
        dest = project_root / script
        if src.exists():
            if dest.exists(): os.remove(dest)
            shutil.copy2(src, dest)
            os.chmod(dest, 0o755)
            print(f"  ✅ Force Updated: {script}")

    # 5. Update Metadata Docs
    for doc in ["CODE_OF_CONDUCT.md", "SECURITY.md", "CONTRIBUTORS", ".gitignore"]:
        src = tools_src / doc
        if doc == ".gitignore": src = tools_src / ".gitignore_template"
        dest = project_root / doc
        if src.exists():
            shutil.copy2(src, dest)
            print(f"  ✅ Updated: {doc}")

    # 6. Ensure unit key exists
    key_src = tools_src / "keys" / "uksfta.bikey"
    key_dest = project_root / "keys" / "uksfta.bikey"
    if key_src.exists():
        project_root.joinpath("keys").mkdir(exist_ok=True)
        shutil.copy2(key_src, key_dest)
        print(f"  ✅ Synced Key: uksfta.bikey")

    print("\nSetup complete! Project is now production-ready.")

if __name__ == "__main__":
    setup_project()
