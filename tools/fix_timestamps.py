import os
import sys
import time
import re
from datetime import datetime, timedelta

def win_filetime_to_datetime(ft):
    # Windows FileTime is 100ns intervals since Jan 1, 1601
    return datetime(1601, 1, 1) + timedelta(microseconds=ft // 10)

def get_win32_timestamp():
    unix_now = time.time()
    return int((unix_now + 11644473600) * 10000000)

def fix_meta_cpp(file_path, project_name=None, published_id=None):
    if not os.path.exists(file_path):
        return
    
    try:
        with open(file_path, "r") as f:
            content = f.read()
        
        # 1. Fix Timestamp
        new_timestamp = get_win32_timestamp()
        content = re.sub(r"timestamp\s*=\s*\d+;", f"timestamp = {new_timestamp};", content)
        
        # 2. Fix Name if provided (ensures launcher consistency)
        if project_name:
            content = re.sub(r'name\s*=\s*".*?";', f'name = "{project_name}";', content)

        # 3. Fix Published ID if provided
        if published_id and str(published_id) != "0":
            if "publishedid" in content.lower():
                content = re.sub(r"publishedid\s*=\s*\d+;", f"publishedid = {published_id};", content)
            else:
                content += f"\npublishedid = {published_id};"
        
        with open(file_path, "w") as f:
            f.write(content)
        
        # Display readable time for log auditing
        readable = win_filetime_to_datetime(new_timestamp).strftime('%Y-%m-%d %H:%M:%S')
        print(f"  Normalized meta.cpp (ID: {published_id}, Time: {new_timestamp} | {readable})")
    except Exception as e:
        print(f"  Warning: Could not update meta.cpp: {e}")

def fix_timestamps(directory, project_name=None, published_id=None):
    if not os.path.exists(directory):
        return
    
    now = time.time()
    count = 0
    for root, dirs, files in os.walk(directory):
        for d in dirs:
            try:
                os.utime(os.path.join(root, d), (now, now))
                count += 1
            except: pass
        for f in files:
            full_path = os.path.join(root, f)
            try:
                if f.lower() == "meta.cpp":
                    fix_meta_cpp(full_path, project_name, published_id)
                os.utime(full_path, (now, now))
                count += 1
            except: pass
    print(f"  Normalized {count} timestamps.")

if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else ".hemttout"
    p_name = sys.argv[2] if len(sys.argv) > 2 else None
    p_id = sys.argv[3] if len(sys.argv) > 3 else None
    
    if os.path.isfile(target):
        if os.path.basename(target).lower() == "meta.cpp":
            fix_meta_cpp(target, p_name, p_id)
        os.utime(target, None)
    else:
        fix_timestamps(target, p_name, p_id)
