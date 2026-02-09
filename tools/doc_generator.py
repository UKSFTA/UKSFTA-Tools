#!/usr/bin/env python3
import os
import sys
import re
from pathlib import Path

def generate_docs(project_path):
    print(f"📖 Generating API Docs for: {os.path.basename(project_path)}")
    
    functions_dir = Path(project_path) / "addons" / "main" / "functions"
    if not functions_dir.exists():
        print("  Functions directory not found.")
        return

    output_md = "# 🛠 UKSFTA Global Function Library\n\n"
    output_md += "Automatically generated API reference for unit-standard functions.\n\n"

    found_any = False
    for f in sorted(functions_dir.glob("fnc_*.sqf")):
        try:
            content = f.read_text(errors='ignore')
            
            # Simple parser for BIS-style headers
            desc = re.search(r'Description:\s*(.*)', content, re.IGNORECASE)
            params = re.findall(r'Parameter:\s*(.*)', content, re.IGNORECASE)
            ret = re.search(r'Return:\s*(.*)', content, re.IGNORECASE)
            
            output_md += f"### `{f.stem}`\n"
            output_md += f"**Description**: {desc.group(1).strip() if desc else 'No description provided.'}\n\n"
            
            if params:
                output_md += "**Parameters**:\n"
                for p in params:
                    output_md += f"- {p.strip()}\n"
                output_md += "\n"
                
            output_md += f"**Returns**: {ret.group(1).strip() if ret else 'Nothing'}\n\n"
            output_md += "---\n\n"
            found_any = True
        except Exception as e:
            print(f"  ⚠️ Error parsing {f.name}: {e}")

    if found_any:
        docs_out = Path(project_path) / "docs" / "API_REFERENCE.md"
        os.makedirs(docs_out.parent, exist_ok=True)
        docs_out.write_text(output_md)
        print(f"  ✅ API Docs generated: {docs_out.relative_to(project_path)}")
    else:
        print("  No functions with 'fnc_' prefix found.")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: doc_generator.py <project_path>")
        sys.exit(1)
    generate_docs(sys.argv[1])
