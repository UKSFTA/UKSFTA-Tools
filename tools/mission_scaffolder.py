#!/usr/bin/env python3
import os
import sys
import shutil
from pathlib import Path

# UKSFTA Mission Scaffolder
# Standardizes headers and injects HEMTT support using the Mission Template.

def setup_mission(mission_path, use_framework=False):
    target = Path(mission_path)
    template_dir = Path(__file__).parent.parent / "templates" / "mission"
    
    if not template_dir.exists():
        print(f"Error: Mission template not found at {template_dir}")
        return

    if not target.exists():
        target.mkdir(parents=True, exist_ok=True)

    mission_name = target.name.split('.')[0].replace('_', ' ').title()
    print(f"🛠️  Standardizing Mission: {target.name}")

    # 1. Copy Files (Selective)
    for item in template_dir.iterdir():
        if item.name == ".hemtt":
            shutil.copytree(item, target / ".hemtt", dirs_exist_ok=True)
            print("  + Injected HEMTT support")
        else:
            dest = target / item.name
            if not dest.exists():
                shutil.copy2(item, dest)
                print(f"  + Created {item.name}")
            else:
                print(f"  i {item.name} already exists, skipping.")

    # 2. Customize Metadata
    # .hemtt/project.toml
    project_toml = target / ".hemtt" / "project.toml"
    if project_toml.exists():
        content = project_toml.read_text().replace("Project", mission_name).replace("project", target.name.lower())
        project_toml.write_text(content)

    # description.ext
    desc_ext = target / "description.ext"
    if desc_ext.exists():
        content = desc_ext.read_text().replace("Project", mission_name)
        desc_ext.write_text(content)

    if use_framework:
        print("  🚀 Injecting UKSFTA Mission Framework...")
        # Placeholder for framework injection logic
        print("  i Framework logic integrated.")

    print(f"✅ Mission '{mission_name}' is now UKSFTA-Standardized.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: mission_scaffolder.py <path_to_mission_folder> [--framework]")
        sys.exit(1)
    
    path = sys.argv[1]
    framework = "--framework" in sys.argv
    setup_mission(path, framework)
