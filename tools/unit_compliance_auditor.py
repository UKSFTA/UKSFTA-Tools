#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
from pathlib import Path

# Soft-import rich
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    USE_RICH = True
except ImportError:
    USE_RICH = False

CORE_FILES = ["build.sh", "release.sh", "bootstrap.sh", "VERSION", "mod.cpp", "meta.cpp", ".hemtt/project.toml", "CODE_OF_CONDUCT.md", "SECURITY.md", "CONTRIBUTORS", ".gitignore"]
CORE_DIRS = [".hemtt/hooks", ".hemtt/scripts", "tools", "addons/main"]

def get_projects():
    parent_dir = Path(__file__).parent.parent.parent.resolve()
    return sorted([d for d in parent_dir.iterdir() if d.is_dir() and d.name.startswith("UKSFTA-")])

def audit_project(proj_path):
    proj_path = Path(proj_path); report = {"present": [], "missing": []}
    for f in CORE_FILES:
        if (proj_path / f).exists(): report["present"].append(f)
        else: report["missing"].append(f)
    for d in CORE_DIRS:
        if (proj_path / d).is_dir() or (proj_path / d).is_symlink(): report["present"].append(d)
        else: report["missing"].append(d)
    total = len(CORE_FILES) + len(CORE_DIRS)
    score = (len(report["present"]) / total) * 100
    return score, report

def main():
    projects = get_projects(); results = []
    print("\n🛡️  [Unit Intelligence] Diamond Standard Compliance Audit")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    for p in projects:
        score, report = audit_project(p)
        results.append({"name": p.name, "score": score, "missing": report["missing"]})
    if USE_RICH:
        console = Console()
        table = Table(box=box.ROUNDED, header_style="bold cyan", border_style="blue")
        table.add_column("Project", style="magenta"); table.add_column("Compliance %", justify="center"); table.add_column("Missing Critical Components")
        for r in sorted(results, key=lambda x: x["score"], reverse=True):
            color = "green" if r["score"] == 100 else ("yellow" if r["score"] >= 80 else "red")
            missing_str = ", ".join(r["missing"][:3]) + ("..." if len(r["missing"]) > 3 else "") if r["missing"] else "[dim]Fully Compliant[/dim]"
            table.add_row(r["name"], f"[{color}]{r['score']:.0f}%[/]", missing_str)
        console.print(table)
    else:
        for r in sorted(results, key=lambda x: x["score"], reverse=True):
            print(f"  {r['name']:<25} | {r['score']:>3.0f}% | Missing: {r['missing'][:3]}")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n")

if __name__ == "__main__": main()
