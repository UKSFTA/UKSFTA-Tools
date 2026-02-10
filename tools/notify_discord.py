#!/usr/bin/env python3
import os
import sys
import json
import urllib.request
import urllib.error

def send_discord_notification(webhook_url, content=None, embed=None):
    if not webhook_url:
        print("Error: No Discord Webhook URL provided.")
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
        with urllib.request.urlopen(req) as response:
            print(f"Notification sent. Status: {response.getcode()}")
    except urllib.error.HTTPError as e:
        print(f"Failed to send notification: {e.code} {e.reason}")
        print(e.read().decode('utf-8'))

def main():
    webhook_url = os.getenv("DISCORD_WEBHOOK")
    if not webhook_url:
        print("Skipping notification: DISCORD_WEBHOOK env var not set.")
        sys.exit(0)

    event = os.getenv("GITHUB_EVENT_NAME", "unknown")
    repo = os.getenv("GITHUB_REPOSITORY", "unknown")
    ref = os.getenv("GITHUB_REF", "unknown")
    workflow = os.getenv("GITHUB_WORKFLOW", "unknown")
    status = os.getenv("JOB_STATUS", "success").lower()
    
    # Determine Color
    color = 0x00FF00 if status == "success" else 0xFF0000 # Green or Red
    
    title = f"{workflow}: {status.upper()}"
    description = f"Repository: **{repo}**
Ref: `{ref}`"
    
    if event == "push":
        commit_msg = os.getenv("COMMIT_MESSAGE", "No commit message")
        description += f"
Commit: {commit_msg}"
        
    embed = {
        "title": title,
        "description": description,
        "color": color,
        "footer": {"text": "UKSFTA DevOps | Platinum Suite"},
        "timestamp": datetime.datetime.utcnow().isoformat()
    }
    
    send_discord_notification(webhook_url, embed=embed)

if __name__ == "__main__":
    import datetime
    main()
