#!/usr/bin/env python3
import subprocess
import os
import sys
import argparse
from datetime import datetime
from pathlib import Path

# UKSFTA Automated Changelog Generator
# Analyzes git history to produce professional-grade Markdown release notes.

def generate_project_changelog(project_path, dry_run=False):
    root = Path(project_path)
    if not (root / ".git").exists():
        return

    # 1. Gather Commits since last tag
    try:
        tag_cmd = ["git", "describe", "--tags", "--abbrev=0"]
        last_tag_res = subprocess.run(tag_cmd, cwd=root, capture_output=True, text=True)
        if last_tag_res.returncode == 0:
            last_tag = last_tag_res.stdout.strip()
            cmd = ["git", "log", f"{last_tag}..HEAD", "--pretty=format:%s"]
        else:
            cmd = ["git", "log", "--pretty=format:%s"]
            last_tag = "Initial"

        res = subprocess.run(cmd, cwd=root, capture_output=True, text=True)
        lines = res.stdout.strip().split("\n")
        
        categories = {
            "feat": "🚀 Features",
            "fix": "🐛 Bug Fixes",
            "docs": "📖 Documentation",
            "chore": "📦 Maintenance",
            "refactor": "🏗️ Refactoring",
            "perf": "⚡ Performance"
        }
        
        output = {k: [] for k in categories}
        other = []
        
        for line in lines:
            if not line: continue
            matched = False
            for prefix in categories:
                if line.lower().startswith(prefix):
                    clean = line.split(":", 1)[1].strip() if ":" in line else line
                    output[prefix].append(clean)
                    matched = True
                    break
            if not matched: other.append(line)

        # 2. Build Markdown
        version = "Unknown"
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

        if dry_run:
            print(f"\n--- [DRY-RUN] Changelog for {root.name} ---")
            print(md)
            print("------------------------------------------")
        else:
            log_path = root / "CHANGELOG.md"
            log_path.write_text(md)
            print(f"  ✅ Generated: {root.name}/CHANGELOG.md")

    except Exception as e:
        print(f"  ❌ Error processing {root.name}: {e}")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Changelog Generator")
    parser.add_argument("target", nargs="?", default=".", help="Project or unit root")
    parser.add_argument("--dry-run", action="store_true", help="Preview changelog in console")
    args = parser.parse_args()
    
    root = Path(args.target)
    if (root / ".git").exists():
        generate_project_changelog(root, args.dry_run)
    else:
        # Scan for subprojects
        for d in root.iterdir():
            if d.is_dir() and (d / ".git").exists():
                generate_project_changelog(d, args.dry_run)

if __name__ == "__main__":
    main()
