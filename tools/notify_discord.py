#!/usr/bin/env python3
import os
import sys
import json
import datetime
import urllib.request
import urllib.error
import argparse

def send_discord_notification(webhook_url, content=None, embed=None):
    if not webhook_url:
        return

    payload = {}
    if content: payload["content"] = content
    if embed: payload["embeds"] = [embed]

    try:
        req = urllib.request.Request(
            webhook_url,
            data=json.dumps(payload).encode('utf-8'),
            headers={'Content-Type': 'application/json', 'User-Agent': 'Mozilla/5.0'}
        )
        with urllib.request.urlopen(req): pass
    except Exception as e:
        print(f"Failed to send notification: {e}")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Discord Notifier")
    parser.add_argument("--message", help="Manual message content")
    parser.add_argument("--type", choices=["update", "release", "alert"], default="update", help="Type of manual notification")
    parser.add_argument("--title", help="Manual embed title")
    args = parser.parse_args()

    webhook_url = os.getenv("DISCORD_WEBHOOK")
    if not webhook_url:
        print("Skipping: DISCORD_WEBHOOK not set.")
        sys.exit(0)

    # 1. Manual Notification Logic
    if args.message or args.title:
        title = args.title if args.title else f"UKSFTA Development {args.type.capitalize()}"
        color = 0x3498db
        if args.type == "release": color = 0x2ecc71
        elif args.type == "alert": color = 0xe74c3c
        
        embed = {
            "title": title,
            "description": args.message if args.message else "System maintenance or update in progress.",
            "color": color,
            "footer": {"text": "UKSFTA DevOps | Manual Notification"},
            "timestamp": datetime.datetime.utcnow().isoformat()
        }
        send_discord_notification(webhook_url, embed=embed)
        print("✅ Manual notification sent.")
        return

    # 2. Automated CI Logic (Fallback)
    event_name = os.getenv("GITHUB_EVENT_NAME", "unknown")
    repo = os.getenv("GITHUB_REPOSITORY", "unknown")
    
    title = f"Event: {event_name}"
    description = f"Repository: **{repo}**"
    url = ""
    color = 0x3498db
    
    event_path = os.getenv("GITHUB_EVENT_PATH")
    payload = {}
    if event_path and os.path.exists(event_path):
        with open(event_path, "r") as f:
            payload = json.load(f)

    if event_name == "push" and os.getenv("GITHUB_REF", "").startswith("refs/tags/"):
        tag = os.getenv("GITHUB_REF", "").replace("refs/tags/", "")
        title = f"🚀 Release Deployed: {tag}"
        description = f"A new version of **{repo}** has been released."
        color = 0x2ecc71
        url = f"https://github.com/{repo}/releases/tag/{tag}"
    elif event_name == "issues":
        action = payload.get("action")
        issue = payload.get("issue", {})
        title = f"🐛 Issue {action.capitalize()}: #{issue.get('number')} {issue.get('title')}"
        description = f"**{repo}**\nUser: {issue.get('user', {}).get('login')}\n\n{issue.get('body', '')[:200]}..."
        url = issue.get("html_url")
        color = 0xe67e22
    elif event_name == "pull_request":
        action = payload.get("action")
        pr = payload.get("pull_request", {})
        title = f"🔀 PR {action.capitalize()}: #{pr.get('number')} {pr.get('title')}"
        description = f"**{repo}**\nUser: {pr.get('user', {}).get('login')}\n\n{pr.get('body', '')[:200]}..."
        url = pr.get("html_url")
        color = 0x9b59b6
        if action == "closed" and pr.get("merged"):
            title = f"🔀 PR Merged: #{pr.get('number')} {pr.get('title')}"
            color = 0x2ecc71
    else:
        print("Skipping non-target event.")
        sys.exit(0)

    embed = {
        "title": title,
        "description": description,
        "url": url,
        "color": color,
        "footer": {"text": "UKSFTA DevOps | Platinum Suite"},
        "timestamp": datetime.datetime.utcnow().isoformat()
    }
    send_discord_notification(webhook_url, embed=embed)

if __name__ == "__main__":
    main()
