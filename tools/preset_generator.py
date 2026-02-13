#!/usr/bin/env python3
import os
import sys
import re
import argparse
from pathlib import Path

# UKSFTA Global HTML Preset Generator
# Consumes all project dependencies to create a master Arma 3 Launcher preset.

HTML_HEADER = """<?xml version="1.0" encoding="utf-8"?>
<html>
  <head>
    <meta name="arma:Type" content="preset" />
    <meta name="arma:PresetName" content="UKSFTA Global Master" />
    <title>Arma 3</title>
    <style>
      body { background: #000; color: #fff; font-family: Segoe UI, Tahoma, Arial; }
      .mod-list { background: #222; padding: 20px; }
      .mod-name { color: #D18F21; font-weight: bold; }
      .mod-id { color: #449EBD; font-size: 0.8em; }
    </style>
  </head>
  <body>
    <h1>UKSFTA Master Preset</h1>
    <div class="mod-list">
      <table>
"""

HTML_FOOTER = """      </table>
    </div>
  </body>
</html>
"""

def generate_preset(root_dir, dry_run=False):
    root = Path(root_dir)
    output_path = root / "uksfta_master_preset.html"
    mod_ids = set()
    mod_info = {} # ID -> Name

    # 1. Scan projects for mod_sources.txt
    for p in root.iterdir():
        if p.is_dir() and p.name.startswith("UKSFTA-"):
            src = p / "mod_sources.txt"
            if src.exists():
                with open(src, 'r') as f:
                    for line in f:
                        if "[ignore]" in line.lower(): break
                        m = re.search(r"(\d{8,})", line)
                        if m:
                            mid = m.group(1)
                            mod_ids.add(mid)
                            if "#" in line:
                                mod_info[mid] = line.split("#", 1)[1].strip()

    # 2. Build HTML
    md_lines = [HTML_HEADER]
    for mid in sorted(mod_ids):
        name = mod_info.get(mid, f"Mod {mid}")
        url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={mid}"
        md_lines.append(f'        <tr data-type="ModContainer">\n')
        md_lines.append(f'          <td class="mod-name">{name}</td>\n')
        md_lines.append(f'          <td><a href="{url}" class="mod-id">Workshop ID: {mid}</a></td>\n')
        md_lines.append(f'        </tr>\n')
    md_lines.append(HTML_FOOTER)
    
    full_html = "".join(md_lines)

    if dry_run:
        print("\n--- [DRY-RUN] Global Preset Preview ---")
        print(f"Found {len(mod_ids)} unique mods.")
        # Print a small sample
        for mid in list(mod_ids)[:5]:
            print(f" • {mod_info.get(mid, mid)}")
        if len(mod_ids) > 5: print(f" ... and {len(mod_ids)-5} more.")
        print("---------------------------------------\n")
    else:
        output_path.write_text(full_html)
        print(f"✅ Global Preset generated at: {output_path}")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Preset Generator")
    parser.add_argument("--dry-run", action="store_true", help="Preview preset content in console")
    args = parser.parse_args()
    
    generate_preset(Path(__file__).parent.parent, args.dry_run)

if __name__ == "__main__":
    main()
