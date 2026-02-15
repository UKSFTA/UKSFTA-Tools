#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
import json
from pathlib import Path

# --- CONFIGURATION ---
TOOLS_ROOT = Path(__file__).parent.parent

def run_tool(cmd):
    res = subprocess.run([sys.executable] + cmd, capture_output=True, text=True)
    return res.stdout

def analyze_unit():
    print(f"
🧠 [THE ARCHITECT] Reasoning Agent v4.0")
    print(f"[*] Analyzing Unit Intelligence...")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    health_data = run_tool(["tools/platinum_score.py"])
    dep_data = run_tool(["tools/dependency_graph.py"])
    perf_data = run_tool(["tools/workspace_manager.py", "audit-performance"])
    actions = []
    if "85%" in health_data or "red" in health_data.lower(): actions.append("🚨 CRITICAL: Unit debt detected. Execute 'unit-wide-sync' immediately.")
    if "CIRCULAR" in dep_data: actions.append("🕸️  STRUCTURAL: Circular dependencies found. Review 'audit-deps' report.")
    if "Warning" in perf_data: actions.append("⚡ PERFORMANCE: High-poly/Large textures detected. Run 'optimize-assets --apply'.")
    if not actions: print("  ✅ ANALYSIS: Unit is operating at Diamond Tier. No actions required.")
    else:
        print("
📝 [STRATEGIC ACTION PLAN]")
        for a in actions: print(f"    - {a}")
    print("
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    print("✨ Reasoning complete. standing by for execution commands.")

if __name__ == "__main__": analyze_unit()
