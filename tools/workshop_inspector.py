#!/usr/bin/env python3
import requests
import json
import os
import sys
from datetime import datetime

# Steam Web API endpoint for Workshop details
STEAM_API_URL = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"

def get_workshop_details(published_ids):
    if not published_ids:
        return []
    
    # Payload for the POST request
    data = {
        "itemcount": len(published_ids),
    }
    for i, pid in enumerate(published_ids):
        data[f"publishedfileids[{i}]"] = pid

    try:
        response = requests.post(STEAM_API_URL, data=data)
        response.raise_for_status()
        return response.json().get("response", {}).get("publishedfiledetails", [])
    except Exception as e:
        print(f"Error querying Steam API: {e}")
        return []

def main():
    print("\n 🔍 UKSFTA Workshop Intelligence")
    print(" ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n")

    # 1. Discover all projects with a workshop ID
    projects = []
    parent_dir = os.path.dirname(os.getcwd())
    
    for folder in os.listdir(parent_dir):
        folder_path = os.path.join(parent_dir, folder)
        toml_path = os.path.join(folder_path, ".hemtt", "project.toml")
        
        if os.path.exists(toml_path):
            with open(toml_path, "r") as f:
                content = f.read()
                import re
                match = re.search(r'workshop_id\s*=\s*"(.*?)"', content)
                if match:
                    wid = match.group(1)
                    if wid and wid != "0":
                        projects.append({
                            "name": folder,
                            "id": wid
                        })

    if not projects:
        print(" [!] No projects with active Workshop IDs found.")
        return

    # 2. Query Steam API
    ids = [p["id"] for p in projects]
    details = get_workshop_details(ids)
    
    # Map details back to projects
    results = []
    for detail in details:
        pid = detail.get("publishedfileid")
        proj = next((p for p in projects if p["id"] == pid), None)
        
        if proj:
            # Result 1 means Success in Steam API
            if detail.get("result") == 1:
                updated = detail.get("time_updated", 0)
                results.append({
                    "name": proj["name"],
                    "id": pid,
                    "title": detail.get("title", "Unknown"),
                    "updated": datetime.fromtimestamp(updated) if updated else "Never",
                    "size": f"{int(detail.get('file_size', 0)) / (1024*1024):.2f} MB",
                    "status": "Public",
                    "link": f"https://steamcommunity.com/sharedfiles/filedetails/?id={pid}"
                })
            else:
                # result 9 usually means access denied (Unlisted/Private)
                results.append({
                    "name": proj["name"],
                    "id": pid,
                    "title": "N/A",
                    "updated": "Check Link",
                    "size": "N/A",
                    "status": "Unlisted/Priv",
                    "link": f"https://steamcommunity.com/sharedfiles/filedetails/?id={pid}"
                })

    # 3. Display Table
    print(f" {'Project':<18} │ {'Status':<14} │ {'Last Updated':<20} │ {'Workshop Link'}")
    print(f" {'─'*18}┼{'─'*16}┼{'─'*22}┼{'─'*50}")
    
    for r in sorted(results, key=lambda x: x['name']):
        status_icon = "✅" if r["status"] == "Public" else "🔗"
        print(f" {r['name']:<18} │ {status_icon} {r['status']:<12} │ {str(r['updated']):<20} │ {r['link']}")

    print("\n [!] Note: 'Unlisted' items may return 'Check Link' due to Steam API restrictions.")
    print("     You can verify them manually using the direct links provided above.")
    print("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

    print("\n ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")

if __name__ == "__main__":
    main()
