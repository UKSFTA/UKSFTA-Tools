#!/usr/bin/env python3
import os
import sys
import json
import datetime
import urllib.request
import urllib.error

def send_discord_notification(webhook_url, content=None, embed=None):
    if not webhook_url:
        return # Silent exit if no webhook

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
    except: pass

def main():
    webhook_url = os.getenv("DISCORD_WEBHOOK")
    if not webhook_url:
        print("Skipping: DISCORD_WEBHOOK not set.")
        sys.exit(0)

    event_name = os.getenv("GITHUB_EVENT_NAME", "unknown")
    repo = os.getenv("GITHUB_REPOSITORY", "unknown")
    
    # Defaults
    title = f"Event: {event_name}"
    description = f"Repository: **{repo}**"
    url = ""
    color = 0x3498db # Blue generic
    
    # Load Event Payload
    event_path = os.getenv("GITHUB_EVENT_PATH")
    payload = {}
    if event_path and os.path.exists(event_path):
        with open(event_path, "r") as f:
            payload = json.load(f)

    # 1. RELEASES
    if event_name == "push" and os.getenv("GITHUB_REF", "").startswith("refs/tags/"):
        tag = os.getenv("GITHUB_REF", "").replace("refs/tags/", "")
        title = f"🚀 Release Deployed: {tag}"
        description = f"A new version of **{repo}** has been released."
        color = 0x2ecc71 # Green
        url = f"https://github.com/{repo}/releases/tag/{tag}"

    # 2. ISSUES
    elif event_name == "issues":
        action = payload.get("action")
        issue = payload.get("issue", {})
        title = f"🐛 Issue {action.capitalize()}: #{issue.get('number')} {issue.get('title')}"
        description = f"**{repo}**\nUser: {issue.get('user', {}).get('login')}\n\n{issue.get('body', '')[:200]}..."
        url = issue.get("html_url")
        color = 0xe67e22 # Orange
        if action == "closed": color = 0x95a5a6 # Gray

    # 3. PULL REQUESTS
    elif event_name == "pull_request":
        action = payload.get("action")
        pr = payload.get("pull_request", {})
        title = f"🔀 PR {action.capitalize()}: #{pr.get('number')} {pr.get('title')}"
        description = f"**{repo}**\nUser: {pr.get('user', {}).get('login')}\n\n{pr.get('body', '')[:200]}..."
        url = pr.get("html_url")
        color = 0x9b59b6 # Purple
        if action == "closed" and pr.get("merged"):
            title = f"🔀 PR Merged: #{pr.get('number')} {pr.get('title')}"
            color = 0x2ecc71
        elif action == "closed":
            color = 0x95a5a6

    else:
        # Ignore other events (like standard push/build)
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
