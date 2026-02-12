#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import re
import urllib.request
import urllib.parse
import argparse
import json

# Try to import rich for high-fidelity CLI output
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    from rich.panel import Panel
    from rich.text import Text
    from rich.columns import Columns
    USE_RICH = True
except ImportError:
    USE_RICH = False

# --- CONFIGURATION & WEIGHTING ---

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
    (r"no key(s)? included", 30),
    (r"not signed", 20),
]

BOTH_PATTERNS = [
    (r"(required|needed|must be) on (both|all|server and client)", 100),
    (r"(must|needs to|required to) be installed on (the )?server and (all )?clients?", 150),
    (r"required on both", 100),
    (r"both server and client", 100),
    (r"sync(hroniz(e|ation))? (is )?required", 80),
    (r"place(able)? in (the )?editor", 50),
    (r"server (needs|requires) (the )?mod", 70),
    (r"signature(s)? (included|required)", 40),
    (r"server (key|bikey)", 40),
    (r"mod (is|must be) signed", 30),
    (r"v\d+\.\d+\.\d+", 10), # Version strings often imply content
]

SERVER_PATTERNS = [
    (r"server[- ]?side (only|mod)", 100),
    (r"not (required|needed) (by|on) clients?", 80),
    (r"clients? (do not|don't) need", 80),
    (r"dedicated server only", 90),
    (r"admin tool|logging", 40),
    (r"server[- ]?side script", 70),
]

CONTENT_TAGS = {
    "weapon": 80, "vehicle": 80, "terrain": 90, "map": 90, "unit": 80,
    "gear": 70, "uniform": 70, "object": 60, "building": 60, "equipment": 80
}

def fetch_workshop_page(published_id):
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36'})
        with urllib.request.urlopen(req, timeout=10) as response:
            return response.read().decode('utf-8')
    except Exception as e:
        return None

def classify_mod(published_id):
    html_content = fetch_workshop_page(published_id)
    if not html_content:
        return None

    title_match = re.search(r'<div class="workshopItemTitle">(.*?)</div>', html_content)
    title = title_match.group(1).strip() if title_match else f"Mod {published_id}"
    
    desc_match = re.search(r'<div class="workshopItemDescription" id="highlightContent">(.*?)</div>', html_content, re.DOTALL)
    description = desc_match.group(1).strip() if desc_match else ""
    clean_desc = re.sub(r'<.*?>', ' ', description)
    clean_desc = re.sub(r'\s+', ' ', clean_desc)
    desc_sentences = [s.strip() for s in re.split(r'[.!?]', clean_desc) if len(s.strip()) > 5]
    
    tags = re.findall(r'requiredtags%5B%5D=(.*?)["\']', html_content)
    tags = sorted(list(set([urllib.parse.unquote(t) for t in tags])))

    scores = {"Client": 0, "Server": 0, "Both": 0}
    evidence = []

    for label, patterns in [("Client", CLIENT_PATTERNS), ("Server", SERVER_PATTERNS), ("Both", BOTH_PATTERNS)]:
        for pattern, weight in patterns:
            matches = re.finditer(pattern, clean_desc, re.IGNORECASE)
            for m in matches:
                scores[label] += weight
                snippet = m.group(0)
                for sentence in desc_sentences:
                    if snippet.lower() in sentence.lower():
                        snippet = sentence
                        break
                evidence.append({"type": label, "text": snippet.strip(), "weight": weight, "pattern": pattern})

    for tag in tags:
        tag_lower = tag.lower()
        for c_tag, weight in CONTENT_TAGS.items():
            if c_tag in tag_lower:
                scores["Both"] += weight
                evidence.append({"type": "Both", "text": f"Mod Tag Match: '{tag}'", "weight": weight})

    total = sum(scores.values())
    summary = []
    for label in ["Client", "Both", "Server"]:
        val = scores[label]
        pct = round((val / total) * 100, 1) if total > 0 else 0.0
        summary.append({"label": label, "score": val, "pct": pct})
    
    summary_sorted = sorted(summary, key=lambda x: x['pct'], reverse=True)
    top = summary_sorted[0] if total > 0 else {"label": "Indecisive", "pct": 0.0}
    
    return {
        "id": published_id,
        "title": title,
        "result": top['label'],
        "confidence": top['pct'],
        "evidence": sorted(evidence, key=lambda x: x['weight'], reverse=True),
        "tags": tags,
        "scores": summary
    }

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Advanced Mod Classifier")
    parser.add_argument("url_or_id", help="Steam Workshop URL or ID")
    parser.add_argument("--json", action="store_true", help="Output result as JSON")
    args = parser.parse_args()

    mid_match = re.search(r"(?:id=)?(\d{8,})", args.url_or_id)
    if not mid_match:
        print("Error: Invalid Workshop ID/URL.")
        sys.exit(1)
    
    mid = mid_match.group(1)
    data = classify_mod(mid)
    if not data:
        print(f"Failed to fetch mod {mid}")
        sys.exit(1)

    if args.json:
        print(json.dumps(data, indent=2))
        return

    console = Console() if USE_RICH else None
    if USE_RICH:
        color = "green" if data['result'] == "Client" else ("yellow" if data['result'] == "Both" else "magenta")
        if data['result'] == "Indecisive": color = "white"
        
        header = Text()
        header.append(f"{data['title']}\n", style="bold white")
        header.append(f"Verdict: {data['result']} ", style=f"bold {color}")
        header.append(f"({data['confidence']}% Confidence)", style="dim")
        console.print(Panel(header, border_style=color, title=f"Mod Audit: {mid}"))

        score_table = Table(box=box.SIMPLE, header_style="bold cyan", title="Classification Metrics", title_justify="left")
        score_table.add_column("Requirement Type")
        score_table.add_column("Evidence Weight", justify="right")
        score_table.add_column("Probability", justify="right")
        for s in data['scores']:
            s_color = "green" if s['label'] == "Client" else ("yellow" if s['label'] == "Both" else "magenta")
            score_table.add_row(f"[{s_color}]{s['label']}[/]", str(s['score']), f"{s['pct']}%")
        console.print(score_table)

        if data['tags']:
            console.print(f"[bold dim]Associated Tags:[/] [italic cyan]{', '.join(data['tags'])}[/italic cyan]")

        if data['evidence']:
            console.print("\n[bold]Technical Evidence Found:[/bold]")
            for e in data['evidence']:
                e_color = "cyan" if e['type'] == "Client" else ("yellow" if e['type'] == "Both" else "magenta")
                console.print(f" • [[bold {e_color}]{e['type']:<6}[/]] {e['text']} [dim](Power: {e['weight']})[/]")
        else:
            console.print("\n[yellow]! Manual verification required: No definitive requirement markers found in description or tags.[/yellow]")

        if data['confidence'] < 50 and data['result'] != "Indecisive":
            console.print("\n[bold red]⚠ AMBIGUITY DETECTED:[/] Classification is based on ambient metadata. Verify in a local environment.")
    else:
        print(f"Verdict: {data['result']} ({data['confidence']}%)")
        for e in data['evidence']: print(f" - [{e['type']}] {e['text']}")

if __name__ == "__main__":
    main()
