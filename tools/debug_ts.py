import urllib.request
import re
import html

def check(mid):
    url = f"https://steamcommunity.com/sharedfiles/filedetails/?id={mid}"
    print(f"Checking {mid}...")
    try:
        req = urllib.request.Request(url, headers={'User-Agent': 'Mozilla/5.0'})
        with urllib.request.urlopen(req, timeout=10) as response:
            page = response.read().decode('utf-8')
            ts_match = re.search(r'data-timestamp="(\d+)"', page)
            if ts_match:
                print(f"  Timestamp Found: {ts_match.group(1)}")
            else:
                print(f"  NO TIMESTAMP FOUND in page content.")
                # print(page[:1000]) # Debug snippet if needed
    except Exception as e:
        print(f"  Error: {e}")

check("3312210548") # CUP UAF
check("2822758266") # Deformer
