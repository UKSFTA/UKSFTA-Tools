#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import subprocess
import re
import json
from pathlib import Path

# --- CONFIGURATION ---
TOOLS_ROOT = Path(__file__).parent.parent
DEBINARIZER = TOOLS_ROOT / "bin" / "linux-x64" / "debinarizer"

RULES = {
    "uniform": {
        "patterns": [r"face_hide", r"hl_uniform", r"unit_main"],
        "proxies": [r"proxy:.*person"],
        "weight": 100
    },
    "vest": {
        "patterns": [r"vest_heavy", r"vest_light", r"pouch_"],
        "proxies": [r"proxy:.*vest"],
        "weight": 80
    },
    "helmet": {
        "patterns": [r"helmet", r"headgear", r"nvg_"],
        "proxies": [r"proxy:.*helmet"],
        "weight": 90
    },
    "weapon": {
        "patterns": [r"zasleh", r"muzzle", r"trigger", r"magazine"],
        "proxies": [r"proxy:.*addon"],
        "weight": 150
    }
}

def classify_asset(p3d_path):
    if not DEBINARIZER.exists():
        return "Unknown (Binary Missing)"

    cmd = [str(DEBINARIZER), str(p3d_path), "-info"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=15)
        output = res.stdout.lower()
        
        scores = {k: 0 for k in RULES.keys()}
        
        for cat, rule in RULES.items():
            # 1. Match Named Selections/Patterns
            for p in rule["patterns"]:
                if re.search(p, output):
                    scores[cat] += rule["weight"]
            
            # 2. Match Proxies
            for p in rule["proxies"]:
                if re.search(p, output):
                    scores[cat] += rule["weight"] * 1.5

        # Determine winner
        best_cat = "Generic Asset"
        best_score = 0
        for cat, score in scores.items():
            if score > best_score:
                best_score = score
                best_cat = cat

        return best_cat.capitalize()
    except Exception as e:
        return f"Classification Error: {e}"

def main():
    if len(sys.argv) < 2:
        print("Usage: asset_classifier.py <file.p3d>")
        sys.exit(1)
    
    p3d = sys.argv[1]
    result = classify_asset(p3d)
    print(f"[*] Asset: {os.path.basename(p3d)}")
    print(f"[*] Classification: {result}")

if __name__ == "__main__":
    main()
