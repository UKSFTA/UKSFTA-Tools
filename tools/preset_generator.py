#!/usr/bin/env python3
import os
import sys
import re
import json
from pathlib import Path

# UKSFTA Global Preset Generator
# Aggregates all mod dependencies from all projects into a single HTML preset.

HTML_HEADER = """<?xml version="1.0" encoding="utf-8"?>
<html>
  <head>
    <meta name="arma:preset-name" content="UKSF Taskforce Alpha - Global Preset" />
    <meta name="arma:last-updated" content="{{DATE}}" />
    <title>Arma 3 Preset</title>
    <style>
      body { background: #1a1a1a; color: #fff; font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif; }
      .container { padding: 20px; }
      h1 { border-bottom: 2px solid #3498db; padding-bottom: 10px; }
      table { width: 100%; border-collapse: collapse; margin-top: 20px; }
      td { padding: 10px; border-bottom: 1px solid #333; }
      .mod-name { font-weight: bold; color: #3498db; }
      .mod-id { color: #888; font-size: 0.8em; }
    </style>
  </head>
  <body>
    <div class="container">
      <h1>UKSF Taskforce Alpha | Global Mod Preset</h1>
      <p>Import this file into your Arma 3 Launcher to synchronize all unit dependencies.</p>
      <table>
"""

HTML_FOOTER = """      </table>
    </div>
  </body>
</html>
"""

def generate_preset(root_dir):
    root = Path(root_dir)
    mod_ids = set()
    mod_info = {}

    print("🔍 Scanning workspace for mod dependencies...")
    
    # Scan all project dirs for mods.lock
    for lock_file in root.parent.glob("UKSFTA-*/mods.lock"):
        try:
            with open(lock_file, 'r') as f:
                data = json.load(f).get("mods", {})
                for mid, info in data.items():
                    mod_ids.add(mid)
                    mod_info[mid] = info.get("name", f"Mod {mid}")
        except: pass

    if not mod_ids:
        print("No mod dependencies found in workspace.")
        return

    output_path = root / "all_releases" / "UKSFTA_Global_Preset.html"
    output_path.parent.mkdir(exist_ok=True)

    date_str = datetime.now().strftime("%d %b %Y") if 'datetime' in sys.modules else "2026"
    
    with open(output_path, 'w') as f:
        f.write(HTML_HEADER.replace("{{DATE}}", date_str))
        for mid in sorted(mod_ids):
            name = mod_info.get(mid, f"Mod {mid}")
            url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={mid}"
            f.write(f'        <tr data-type="ModContainer">\n')
            f.write(f'          <td class="mod-name">{name}</td>\n')
            f.write(f'          <td><a href="{url}" class="mod-id">Workshop ID: {mid}</a></td>\n')
            f.write(f'        </tr>\n')
        f.write(HTML_FOOTER)

    print(f"✅ Global Preset generated at: {output_path}")

if __name__ == "__main__":
    from datetime import datetime
    generate_preset(Path(__file__).parent.parent)
