#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import urllib.request
import argparse
import json

# Try to import rich for high-fidelity CLI output
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    from rich.panel import Panel
    from rich.text import Text
    USE_RICH = True
except ImportError:
    USE_RICH = False

# --- CONFIGURATION & WEIGHTING ---

# Keywords suggesting Client-Side Only (Visuals, HUDs, local sounds)
CLIENT_PATTERNS = [
    (r"client[- ]?side (only|mod)", 100),
    (r"not (required|needed) on (the )?server", 80),
    (r"optional (for|on) (the )?server", 70),
    (r"visuals? only", 60),
    (r"sounds? only", 50),
    (r"interface|hud|ui only", 60),
    (r"local(ly)? only", 40),
    (r"no server[- ]?side (needed|req)", 80),
    (r"work(s)? without the mod on the server", 90),
]

# Keywords suggesting Both (Content, Weapons, Vehicles, Synchronized scripts)
BOTH_PATTERNS = [
    (r"(required|needed|must be) on (both|all|server and client)", 100),
    (r"sync(hroniz(e|ation))? (is )?required", 80),
    (r"place(able)? in (the )?editor", 50),
    (r"server (needs|requires) (the )?mod", 70),
    (r"signature(s)? (included|required)", 40),
    (r"server (key|bikey)", 40),
    (r"mod (is|must be) signed", 30),
]

# Keywords suggesting Server-Side Only (Admin tools, logging, AI behavior)
SERVER_PATTERNS = [
    (r"server[- ]?side (only|mod)", 100),
    (r"not (required|needed) (by|on) clients?", 80),
    (r"clients? (do not|don't) need", 80),
    (r"dedicated server only", 90),
    (r"admin tool|logging", 40),
]

# Content Tags (If these exist, it's almost certainly "Both")
CONTENT_TAGS = {
    "weapon": 80, "vehicle": 80, "terrain": 90, "map": 90, "unit": 80,
    "gear": 70, "uniform": 70, "object": 60, "building": 60
}

def fetch_workshop_page(published_id):
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36'})
        with urllib.request.urlopen(req, timeout=10) as response:
            return response.read().decode('utf-8')
    except Exception as e:
        print(f"Error fetching mod page: {e}")
        return None

def classify_mod(published_id):
    html_content = fetch_workshop_page(published_id)
    if not html_content:
        return None

    # Extract Metadata
    title_match = re.search(r'<div class="workshopItemTitle">(.*?)</div>', html_content)
    title = title_match.group(1).strip() if title_match else f"Mod {published_id}"
    
    desc_match = re.search(r'<div class="workshopItemDescription" id="highlightContent">(.*?)</div>', html_content, re.DOTALL)
    description = desc_match.group(1).strip() if desc_match else ""
    # Clean description for regex
    clean_desc = re.sub(r'<.*?>', ' ', description)
    # Get lines for snippet extraction
    desc_lines = [l.strip() for l in clean_desc.split('.') if l.strip()]
    
    tags = re.findall(r'href="https://steamcommunity.com/workshop/browse/\?appid=107410&.*?requiredtags%5B%5D=(.*?)">', html_content)
    tags = [t.lower() for t in tags]

    # Analysis Tally
    scores = {"Client": 0, "Server": 0, "Both": 0}
    evidence = []

    # 1. Run Regex Patterns
    for label, patterns in [("Client", CLIENT_PATTERNS), ("Server", SERVER_PATTERNS), ("Both", BOTH_PATTERNS)]:
        for pattern, weight in patterns:
            matches = re.finditer(pattern, clean_desc, re.IGNORECASE)
            for m in matches:
                scores[label] += weight
                # Find the sentence containing the match
                snippet = m.group(0)
                for line in desc_lines:
                    if snippet.lower() in line.lower():
                        snippet = line
                        break
                evidence.append({"type": label, "text": snippet.strip(), "weight": weight})

    # 2. Analyze Tags
    for tag in tags:
        for c_tag, weight in CONTENT_TAGS.items():
            if c_tag in tag:
                scores["Both"] += weight
                evidence.append({"type": "Both", "text": f"Mod Tag: '{tag}' (Implies synchronized content)", "weight": weight})

    # 3. Calculate Confidence
    total = sum(scores.values())
    if total == 0:
        return {"title": title, "result": "Indecisive", "confidence": 0, "evidence": []}

    # Normalize
    results = []
    for label, val in scores.items():
        results.append({"label": label, "pct": (val / total) * 100})
    
    # Sort by percentage
    results = sorted(results, key=lambda x: x['pct'], reverse=True)
    
    # Winner
    top = results[0]
    
    return {
        "title": title,
        "result": top['label'],
        "confidence": round(top['pct'], 1),
        "evidence": sorted(evidence, key=lambda x: x['weight'], reverse=True)[:5],
        "all_scores": results
    }

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Fuzzy Mod Classifier")
    parser.add_argument("url_or_id", help="Steam Workshop URL or ID")
    parser.add_argument("--json", action="store_true", help="Output result as JSON")
    args = parser.parse_args()

    mid_match = re.search(r"(?:id=)?(\d{8,})", args.url_or_id)
    if not mid_match:
        print("Error: Invalid Workshop ID/URL.")
        sys.exit(1)
    
    mid = mid_match.group(1)
    data = classify_mod(mid)
    if not data: sys.exit(1)

    if args.json:
        print(json.dumps(data, indent=2))
        return

    console = Console() if USE_RICH else None
    if USE_RICH:
        color = "green" if data['result'] == "Client" else ("yellow" if data['result'] == "Both" else "magenta")
        if data['result'] == "Indecisive": color = "white"
        
        content = Text()
        content.append(f"{data['title']}\n", style="bold white")
        content.append(f"{data['result']} ", style=f"bold {color}")
        content.append(f"({data['confidence']}% Confidence)", style="dim")
        
        console.print(Panel(content, border_style=color, title="Classification Result"))
        
        if data['evidence']:
            console.print("\n[bold dim]Primary Evidence:[/bold dim]")
            for e in data['evidence']:
                e_color = "cyan" if e['type'] == "Client" else ("yellow" if e['type'] == "Both" else "magenta")
                console.print(f" • [[bold {e_color}]{e['type']}[/]] {e['text']}")
        
        if data['confidence'] < 50 and data['result'] != "Indecisive":
            console.print("\n[bold red]⚠ LOW CONFIDENCE:[/] Manual testing is highly recommended.")
    else:
        print(f"Mod: {data['title']}")
        print(f"Result: {data['result']} ({data['confidence']}%)")
        for e in data['evidence']:
            print(f" - [{e['type']}] {e['text']}")

if __name__ == "__main__":
    main()
