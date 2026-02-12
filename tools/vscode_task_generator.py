#!/usr/bin/env python3
import os
import json
from pathlib import Path

# UKSFTA VS Code Task Generator
# Generates .vscode/tasks.json for one-click dev actions.

TASKS = {
    "version": "2.0.0",
    "tasks": [
        {
            "label": "🛡️  UKSF: Run Master Audit",
            "type": "shell",
            "command": "python3 .uksf_tools/tools/workspace_manager.py audit",
            "group": {"kind": "test", "isDefault": True},
            "problemMatcher": []
        },
        {
            "label": "🏗️  UKSF: Build Project",
            "type": "shell",
            "command": "bash build.sh build",
            "group": {"kind": "build", "isDefault": True},
            "problemMatcher": []
        },
        {
            "label": "🔄  UKSF: Sync Mods",
            "type": "shell",
            "command": "python3 .uksf_tools/tools/manage_mods.py sync",
            "problemMatcher": []
        },
        {
            "label": "🧹  UKSF: Fix Syntax",
            "type": "shell",
            "command": "python3 .uksf_tools/tools/syntax_fixer.py .",
            "problemMatcher": []
        },
        {
            "label": "📊  UKSF: Size Report",
            "type": "shell",
            "command": "python3 .uksf_tools/tools/size_reporter.py",
            "problemMatcher": []
        }
    ]
}

def generate_vscode_config(project_path):
    root = Path(project_path)
    vscode_dir = root / ".vscode"
    vscode_dir.mkdir(exist_ok=True)
    
    tasks_file = vscode_dir / "tasks.json"
    with open(tasks_file, "w") as f:
        json.dump(TASKS, f, indent=4)
    
    print(f"  ✅ Generated .vscode/tasks.json in: {project_path}")

if __name__ == "__main__":
    import sys
    target = sys.argv[1] if len(sys.argv) > 1 else "."
    generate_vscode_config(target)
