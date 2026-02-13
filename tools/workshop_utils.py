# -*- coding: utf-8 -*-
import re
import urllib.request
import urllib.parse
import json
import html

STEAM_API_URL = "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/"
IGNORED_APP_IDS = {"107410", "228800"}

def get_bulk_metadata(published_ids):
    """Fetches metadata for multiple mods via Steam API (Fast)."""
    if not published_ids: return {}
    results = {}
    id_list = list(published_ids)
    for i in range(0, len(id_list), 100):
        chunk = id_list[i:i+100]
        data = {"itemcount": len(chunk)}
        for j, pid in enumerate(chunk): data[f"publishedfileids[{j}]"] = pid
        try:
            encoded_data = urllib.parse.urlencode(data).encode('utf-8')
            req = urllib.request.Request(STEAM_API_URL, data=encoded_data, method='POST')
            with urllib.request.urlopen(req, timeout=10) as response:
                res = json.loads(response.read().decode('utf-8'))
                details = res.get("response", {}).get("publishedfiledetails", [])
                for d in details:
                    mid = d.get("publishedfileid")
                    if mid:
                        results[mid] = {
                            "name": d.get("title", f"Mod {mid}"),
                            "updated": str(d.get("time_updated", "0")),
                            "size": int(d.get("file_size", 0)),
                            "creator_id": d.get("creator"),
                            "dependencies": []
                        }
        except: pass
    return results

def scrape_required_items(published_id):
    """Scrapes 'Required Items' from the mod's Workshop page."""
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={published_id}"
    req_ids = set()
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as response:
            page = response.read().decode('utf-8')
            matches = re.findall(r'class="requiredItem".*?id=(\d+)', page, re.DOTALL)
            for m in matches:
                if m not in IGNORED_APP_IDS: req_ids.add(m)
    except: pass
    return req_ids

def resolve_transitive_dependencies(initial_ids, all_acknowledged_ids):
    """
    Recursively discovers dependencies for a set of mods.
    Returns a dict of mid -> metadata.
    """
    resolved_info = {}
    to_check = list(initial_ids)
    processed = set()
    found_as_dep = set()
    
    # Warmup API cache
    api_cache = get_bulk_metadata(to_check)

    while to_check:
        # Resolve names for new transitive finds
        unresolved = [mid for mid in to_check if mid not in api_cache and mid not in processed]
        if unresolved:
            api_cache.update(get_bulk_metadata(unresolved))

        mid = to_check.pop(0)
        if mid in processed or mid in IGNORED_APP_IDS: continue

        meta = api_cache.get(mid, {"name": f"Mod {mid}", "updated": "0", "size": 0, "dependencies": []})
        found_deps = scrape_required_items(mid)
        
        # Determine unique new deps to follow
        meta["dependencies"] = []
        for fid in found_deps:
            if fid not in all_acknowledged_ids and fid not in processed and fid not in to_check:
                to_check.append(fid)
                found_as_dep.add(fid)
            meta["dependencies"].append({"id": fid, "name": f"Mod {fid}"}) # Resolved later

        resolved_info[mid] = meta
        processed.add(mid)
        
    # Final pass: Fill in names for dependency lists in metadata
    for mid, info in resolved_info.items():
        for dep in info["dependencies"]:
            if dep["id"] in resolved_info:
                dep["name"] = resolved_info[dep["id"]]["name"]
            elif dep["id"] in api_cache:
                dep["name"] = api_cache[dep["id"]]["name"]

    return resolved_info
