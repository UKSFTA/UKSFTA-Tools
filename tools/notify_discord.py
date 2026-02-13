#!/usr/bin/env python3
import os
import sys
import json
import datetime
import urllib.request
import urllib.error
import argparse
import math

def format_size(size_bytes):
    if size_bytes == 0: return "0 B"
    size_name = ("B", "KB", "MB", "GB", "TB")
    i = int(math.floor(math.log(size_bytes, 1024)))
    p = math.pow(1024, i)
    s = round(size_bytes / p, 2)
    return "%s %s" % (s, size_name[i])

def send_discord_notification(webhook_url, content=None, embed=None, dry_run=False):
    payload = {}
    if content: payload["content"] = content
    if embed: payload["embeds"] = [embed]
    
    if dry_run:
        print("\n--- 🕵️ DISCORD DRY-RUN PREVIEW ---")
        if HAS_RICH:
            from rich.syntax import Syntax
            json_str = json.dumps(payload, indent=2)
            syntax = Syntax(json_str, "json", theme="monokai", line_numbers=True)
            Console().print(syntax)
        else:
            print(json.dumps(payload, indent=2))
        print("---------------------------------\n")
        return

    if not webhook_url: return
    try:
        req = urllib.request.Request(webhook_url, data=json.dumps(payload).encode('utf-8'), headers={'Content-Type': 'application/json', 'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req): pass
    except Exception as e: print(f"Failed: {e}")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Discord Notifier")
    parser.add_argument("--message")
    parser.add_argument("--type", choices=["update", "release", "alert"], default="update")
    parser.add_argument("--title")
    parser.add_argument("--impact", help="JSON string of modpack impact report")
    parser.add_argument("--dry-run", action="store_true", help="Preview notification without sending")
    args = parser.parse_args()

    webhook_url = os.getenv("DISCORD_WEBHOOK")
    if not webhook_url and not args.dry_run:
        print("Skipping: DISCORD_WEBHOOK not set.")
        sys.exit(0)

    # 1. High-Fidelity Impact Reporting
    if args.impact:
        impact = json.loads(args.impact)
        project_name = os.path.basename(os.getcwd())
        
        embed = {
            "title": f"📦 Modpack Expansion: {project_name}",
            "description": f"New content has been integrated into the **{project_name}** repository.",
            "color": 0x3498db,
            "fields": [],
            "footer": {"text": "UKSFTA DevOps | Modset Intelligence"},
            "timestamp": datetime.datetime.utcnow().isoformat()
        }

        if impact["added"]:
            added_list = ""
            for mod in impact["added"]:
                added_list += f" • **{mod['name']}** ({format_size(mod['size'])})\n"
                if mod["deps"]:
                    deps = ", ".join(mod["deps"][:5]) + ("..." if len(mod["deps"]) > 5 else "")
                    added_list += f"   [dim]Deps: {deps}[/dim]\n"
            embed["fields"].append({"name": "🆕 Added Content", "value": added_list, "inline": False})

        if impact["removed"]:
            removed_list = ", ".join(impact["removed"])
            embed["fields"].append({"name": "🗑️ Removed Content", "value": removed_list, "inline": False})

        # Summary Metrics
        stats = f"**New Data:** {format_size(impact['added_size'])}\n"
        stats += f"**Total Modset Size:** {format_size(impact['total_size'])}"
        embed["fields"].append({"name": "📊 Resource Impact", "value": stats, "inline": True})

        send_discord_notification(webhook_url, embed=embed, dry_run=args.dry_run)
        if not args.dry_run: print("✅ Impact notification sent.")
        return

    # 2. Manual/Generic Logic
    if args.message or args.title:
        title = args.title if args.title else f"UKSFTA Development {args.type.capitalize()}"
        color = 0x3498db
        if args.type == "release": color = 0x2ecc71
        elif args.type == "alert": color = 0xe74c3c
        embed = {"title": title, "description": args.message or "Update in progress.", "color": color, "footer": {"text": "UKSFTA DevOps"}, "timestamp": datetime.datetime.utcnow().isoformat()}
        send_discord_notification(webhook_url, embed=embed, dry_run=args.dry_run)
        if not args.dry_run: print("✅ Manual notification sent.")
        return

if __name__ == "__main__":
    main()
