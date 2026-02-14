#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import subprocess
import json
import sys
from pathlib import Path

# Soft-import rich
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    from rich.panel import Panel
    USE_RICH = True
except ImportError:
    USE_RICH = False

def get_projects():
    parent_dir = Path(__file__).parent.parent.parent
    return [d for d in parent_dir.iterdir() if d.is_dir() and d.name.startswith("UKSFTA-") and (d / ".hemtt" / "project.toml").exists()]

def calculate_score(project_path):
    project_path = Path(project_path)
    score = 100
    deductions = []

    # 1. Forensic Audit (LODs)
    res_lods = subprocess.run([sys.executable, "tools/asset_auditor.py", str(project_path)], capture_output=True, text=True)
    if "MISSING GEOMETRY" in res_lods.stdout:
        score -= 20
        deductions.append("Missing Geometry LODs (-20)")
    if "External Leaks Detected" in res_lods.stdout:
        score -= 15
        deductions.append("External VFS Path Leaks (-15)")

    # 2. Key Audit
    res_keys = subprocess.run([sys.executable, "tools/key_auditor.py", str(project_path)], capture_output=True, text=True)
    if "Missing official UKSFTA public key" in res_keys.stdout:
        score -= 25
        deductions.append("Missing Unit Public Key (-25)")

    # 3. Performance Audit
    res_perf = subprocess.run([sys.executable, "tools/weight_reporter.py", str(project_path)], capture_output=True, text=True)
    if "High-Poly Warning" in res_perf.stdout:
        score -= 10
        deductions.append("High-Poly Bottlenecks (-10)")
    if "Large Texture Warning" in res_perf.stdout:
        score -= 10
        deductions.append("Oversized Texture Bottlenecks (-10)")

    return max(0, score), deductions

def main():
    projects = get_projects()
    results = []
    
    header = "\n🏆  [Strategic Command] Platinum Health Dashboard"
    separator = " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print(header)
    print(separator)

    for p in projects:
        score, issues = calculate_score(p)
        results.append({"project": p.name, "score": score, "issues": issues})

    if USE_RICH:
        console = Console()
        table = Table(box=box.ROUNDED, header_style="bold magenta", border_style="blue")
        table.add_column("Project", style="cyan")
        table.add_column("Platinum Score", justify="center")
        table.add_column("Critical Deficits")
        
        for r in sorted(results, key=lambda x: x["score"]):
            color = "green" if r["score"] >= 90 else ("yellow" if r["score"] >= 70 else "red")
            issues_str = ", ".join(r["issues"]) if r["issues"] else "[dim]Fully Compliant[/dim]"
            table.add_row(r["project"], f"[{color}]{r['score']}%[/]", issues_str)
        console.print(table)
    else:
        for r in sorted(results, key=lambda x: x["score"]):
            print(f"  {r['project']:<20} | Score: {r['score']}% | {r['issues']}")

    print(separator + "\n")

if __name__ == "__main__":
    main()
