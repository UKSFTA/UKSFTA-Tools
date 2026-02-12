#!/usr/bin/env python3
import os
import sys
import re
from pathlib import Path

# UKSFTA Living API Manual Generator
# Scans all addons for BIS-style headers and generates a categorized Markdown manual.

def parse_header(content):
    details = {
        "description": "No description provided.",
        "params": [],
        "returns": "Nothing",
        "example": None
    }
    
    # Precise extraction
    desc_match = re.search(r'Description:\s*([^\n]*)', content, re.IGNORECASE)
    if desc_match: details["description"] = desc_match.group(1).strip()
    
    params = re.findall(r'Parameter\(s\)?:\s*([^\n]*)', content, re.IGNORECASE)
    if not params: params = re.findall(r'Arguments:\s*([^\n]*)', content, re.IGNORECASE)
    details["params"] = [p.strip() for p in params if p.strip()]
    
    ret_match = re.search(r'Return:\s*([^\n]*)', content, re.IGNORECASE)
    if ret_match: details["returns"] = ret_match.group(1).strip()
    
    # Look for example block
    example_match = re.search(r'Example:\s*\n?\s*(.*?)(?=\n\n|\*\/)', content, re.IGNORECASE | re.DOTALL)
    if example_match: details["example"] = example_match.group(1).strip()
    
    return details

def generate_docs(project_path):
    root = Path(project_path)
    print(f"📖 Generating Master API Manual for: {root.name}")
    
    output_md = f"# 🛠 UKSFTA Function Library: {root.name}\n\n"
    output_md += "Automatically generated API reference for unit components.\n\n"
    
    addons_dir = root / "addons"
    if not addons_dir.exists(): return

    found_functions = {} # Category -> [Function Metadata]

    # Recursive scan for all function files
    for sqf in addons_dir.rglob("fn_*.sqf"):
        # Determine category (component name)
        # Path is usually addons/<component>/functions/fn_name.sqf
        parts = sqf.relative_to(addons_dir).parts
        category = parts[0].capitalize()
        
        if category not in found_functions: found_functions[category] = []
        
        try:
            content = sqf.read_text(errors='ignore')
            data = parse_header(content)
            data["name"] = sqf.stem.replace("fn_", "uksf_") # Assuming unit prefix
            found_functions[category].append(data)
        except Exception as e:
            print(f"  ⚠️ Error parsing {sqf.name}: {e}")

    if not found_functions:
        print("  No functions found.")
        return

    # Render Markdown
    for cat in sorted(found_functions.keys()):
        output_md += f"## 📦 Component: {cat}\n\n"
        for func in sorted(found_functions[cat], key=lambda x: x['name']):
            output_md += f"### `{func['name']}`\n"
            output_md += f"{func['description']}\n\n"
            
            if func['params']:
                output_md += "**Arguments:**\n"
                for p in func['params']: output_md += f"- {p}\n"
                output_md += "\n"
            
            output_md += f"**Returns:** {func['returns']}\n\n"
            
            if func['example']:
                output_md += "**Example:**\n"
                output_md += f"```sqf\n{func['example']}\n```\n\n"
            
            output_md += "---\n\n"

    docs_out = root / "docs" / "FUNCTION_LIBRARY.md"
    os.makedirs(docs_out.parent, exist_ok=True)
    docs_out.write_text(output_md)
    print(f"  ✅ Manual generated: {docs_out.relative_to(project_path)}")

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: doc_generator.py <project_path>")
        sys.exit(1)
    generate_docs(sys.argv[1])
