#!/usr/bin/env python3
import os
import sys
import subprocess
from pathlib import Path
from datetime import datetime

# UKSFTA Professional Changelog Generator
# Generates CHANGELOG.md from git history using conventional commits.

def generate_project_changelog(project_path):
    root = Path(project_path)
    print(f"📄 Generating Changelog for: {root.name}")
    
    try:
        # Get history since last tag (or all time if no tags)
        try:
            last_tag = subprocess.check_output(["git", "describe", "--tags", "--abbrev=0"], cwd=root).decode().strip()
            cmd = ["git", "log", f"{last_tag}..HEAD", "--oneline", "--no-merges"]
        except:
            cmd = ["git", "log", "--oneline", "--no-merges"]
            last_tag = "Initial"

        res = subprocess.run(cmd, cwd=root, capture_output=True, text=True)
        lines = res.stdout.strip().split("\n")
        
        categories = {
            "feat": "🚀 Features",
            "fix": "🐛 Bug Fixes",
            "docs": "📖 Documentation",
            "perf": "⚡ Performance",
            "chore": "🔧 Maintenance",
            "refactor": "🏗️  Refactor"
        }
        
        output = {cat: [] for cat in categories}
        other = []

        for line in lines:
            if not line: continue
            # Split ID from message
            parts = line.split(" ", 1)
            if len(parts) < 2: continue
            msg = parts[1]
            
            found = False
            for prefix, label in categories.items():
                if msg.lower().startswith(f"{prefix}:"):
                    output[prefix].append(msg[len(prefix)+1:].strip())
                    found = True
                    break
            if not found:
                other.append(msg)

        # Build Markdown
        version = "Unreleased"
        v_file = root / "VERSION"
        if v_file.exists(): version = v_file.read_text().strip()
        
        md = f"# Changelog: {root.name}\n\n"
        md += f"## [{version}] - {datetime.now().strftime('%Y-%m-%d')}\n\n"
        
        for prefix, label in categories.items():
            if output[prefix]:
                md += f"### {label}\n"
                for item in sorted(output[prefix]):
                    md += f"- {item}\n"
                md += "\n"
        
        if other:
            md += "### 📦 Other Changes\n"
            for item in sorted(other):
                md += f"- {item}\n"
            md += "\n"

        log_path = root / "CHANGELOG.md"
        log_path.write_text(md)
        print(f"  ✅ Generated: {log_path.name}")
        
    except Exception as e:
        print(f"  ❌ Error generating changelog: {e}")

if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else "."
    generate_project_changelog(target)
