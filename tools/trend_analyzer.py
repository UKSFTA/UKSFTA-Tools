#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import json
import sys
from datetime import datetime
from pathlib import Path
from platinum_score import calculate_score, get_projects

HISTORY_FILE = Path(__file__).parent.parent / "health_history.json"

def save_snapshot():
    projects = get_projects()
    snapshot = {
        "timestamp": datetime.now().isoformat(),
        "projects": {}
    }
    
    print("\n📈 [Trend Analyzer] Capturing Health Snapshot...")
    
    for p in projects:
        score, issues = calculate_score(p)
        snapshot["projects"][p.name] = {
            "score": score,
            "issues_count": len(issues)
        }
        print(f"  - {p.name}: {score}%")

    history = []
    if HISTORY_FILE.exists():
        try:
            with open(HISTORY_FILE, 'r') as f:
                history = json.load(f)
        except: pass
    
    history.append(snapshot)
    
    # Keep last 50 snapshots
    if len(history) > 50:
        history = history[-50:]
        
    with open(HISTORY_FILE, 'w') as f:
        json.dump(history, f, indent=2)
        
    print(f"✅ Snapshot saved to {HISTORY_FILE}")

def report_trends():
    if not HISTORY_FILE.exists():
        print("No history found. Run 'save_snapshot' first.")
        return

    with open(HISTORY_FILE, 'r') as f:
        history = json.load(f)
        
    if len(history) < 2:
        print("Insufficient data for trend analysis.")
        return

    latest = history[-1]
    previous = history[-2]
    
    header = "\n📉 [Trend Report] Health Delta"
    separator = " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print(header)
    print(separator)
    
    for proj_name, data in latest["projects"].items():
        prev_data = previous["projects"].get(proj_name, {"score": 0})
        delta = data["score"] - prev_data["score"]
        
        icon = "➡️"
        if delta > 0: icon = "⬆️"
        elif delta < 0: icon = "⬇️"
        
        print(f"  {proj_name:<20} | {data['score']}% ({icon} {delta:+})")

if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "report":
        report_trends()
    else:
        save_snapshot()
