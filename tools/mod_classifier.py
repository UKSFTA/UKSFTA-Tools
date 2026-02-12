#!/usr/bin/env python3
import os
import sys
import re
import urllib.request
import argparse

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

# Configuration for detection
CLIENT_ONLY_KEYWORDS = [
    r"client[- ]?side only",
    r"optional for server",
    r"no server[- ]?side needed",
    r"visual only",
    r"sound only",
    r"interface only",
    r"hud only",
    r"local only",
    r"not required on server"
]

SERVER_ONLY_KEYWORDS = [
    r"server[- ]?side only",
    r"no client[- ]?side needed",
    r"not required by clients",
    r"script only",
    r"server[- ]?side script"
]

BOTH_KEYWORDS = [
    r"required on both",
    r"must be on server",
    r"synchronization required",
    r"signed with key",
    r"server key included",
    r"installed on the Server and all Clients",
    r"needed on server and client",
    r"required for server and client"
]

CONTENT_TAGS = ["vehicle", "weapon", "terrain", "map", "unit", "uniform", "gear", "equipment"]

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
    clean_desc = re.sub(r'<.*?>', ' ', description)
    
    tags = re.findall(r'href="https://steamcommunity.com/workshop/browse/\?appid=107410&.*?section=readytouseitems&requiredtags%5B%5D=(.*?)">', html_content)
    tags = [t.lower() for t in tags]

    # Scoring Logic
    client_score = 0
    server_score = 0
    both_score = 0
    mentions = []

    for kw in CLIENT_ONLY_KEYWORDS:
        matches = re.findall(f"({kw})", clean_desc, re.IGNORECASE)
        if matches:
            client_score += 40 * len(matches)
            mentions.append(f"[bold cyan]Client-Side Mention:[/bold cyan] '...{matches[0]}...'")

    for kw in SERVER_ONLY_KEYWORDS:
        matches = re.findall(f"({kw})", clean_desc, re.IGNORECASE)
        if matches:
            server_score += 40 * len(matches)
            mentions.append(f"[bold magenta]Server-Side Mention:[/bold magenta] '...{matches[0]}...'")

    for kw in BOTH_KEYWORDS:
        matches = re.findall(f"({kw})", clean_desc, re.IGNORECASE)
        if matches:
            both_score += 30 * len(matches)
            mentions.append(f"[bold yellow]Both-Sides Mention:[/bold yellow] '...{matches[0]}...'")

    for tag in tags:
        if any(ct in tag for t in CONTENT_TAGS for ct in [t]):
            both_score += 50
            mentions.append(f"[dim]Tag Match:[/dim] '{tag}' (Content suggests Both)")

    total = client_score + server_score + both_score
    if total == 0:
        return {"title": title, "result": "Indecisive", "confidence": 0, "mentions": ["No definitive keywords found."]}

    client_pct = (client_score / total) * 100
    server_pct = (server_score / total) * 100
    both_pct = (both_score / total) * 100

    if both_pct > 60:
        result = "Required on Both"
        conf = both_pct
    elif client_pct > server_pct:
        result = "Likely Client-Side Only"
        conf = client_pct
    else:
        result = "Likely Server-Side Only"
        conf = server_pct

    return {
        "title": title,
        "result": result,
        "confidence": round(conf, 1),
        "mentions": mentions[:5]
    }

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Mod Side Classifier")
    parser.add_argument("url_or_id", help="Steam Workshop URL or ID")
    args = parser.parse_args()

    mid_match = re.search(r"(?:id=)?(\d{8,})", args.url_or_id)
    if not mid_match:
        print("Error: Invalid Workshop ID/URL.")
        sys.exit(1)
    
    mid = mid_match.group(1)
    console = Console() if USE_RICH else None
    
    if USE_RICH: console.print(f"\n🔍 [bold blue]Analyzing:[/bold blue] {mid}...")
    else: print(f"\nAnalyzing Mod: {mid}...")

    data = classify_mod(mid)
    if not data: sys.exit(1)

    if USE_RICH:
        color = "green" if "Client" in data['result'] else ("yellow" if "Both" in data['result'] else "magenta")
        content = Text()
        content.append(f"{data['title']}\n", style="bold white")
        content.append(f"{data['result']} ", style=f"bold {color}")
        content.append(f"({data['confidence']}% Confidence)", style="dim")
        
        console.print(Panel(content, border_style=color, title="Classification Result"))
        if data['mentions']:
            console.print("\n[bold dim]Key Mentions:[/bold dim]")
            for m in data['mentions']: console.print(f" • {m}")
    else:
        print(f"Result: {data['result']} ({data['confidence']}% Confidence)")
        for m in data['mentions']: print(f" - {m}")

if __name__ == "__main__":
    main()
