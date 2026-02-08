import os
import sys
import time
import re

def get_win32_timestamp():
    # Convert current Unix time to Win32 FileTime (100-nanosecond intervals since 1601)
    unix_now = time.time()
    return int((unix_now + 11644473600) * 10000000)

def fix_meta_cpp(file_path):
    if not os.path.exists(file_path):
        return
    
    try:
        with open(file_path, "r") as f:
            content = f.read()
        
        # Replace the timestamp = ...; line with a valid Win32 timestamp
        new_timestamp = get_win32_timestamp()
        new_content = re.sub(r"timestamp\s*=\s*\d+;", f"timestamp = {new_timestamp};", content)
        
        with open(file_path, "w") as f:
            f.write(new_content)
        print(f"  Rewrote meta.cpp timestamp to: {new_timestamp}")
    except Exception as e:
        print(f"  Warning: Could not update meta.cpp content: {e}")

def fix_timestamps(directory):
    if not os.path.exists(directory):
        return
    
    now = time.time()
    print(f"Normalizing timestamps in: {directory}")
    
    count = 0
    for root, dirs, files in os.walk(directory):
        for d in dirs:
            try:
                full_path = os.path.join(root, d)
                os.utime(full_path, (now, now))
                count += 1
            except:
                pass
        for f in files:
            full_path = os.path.join(root, f)
            try:
                # If it's a meta.cpp, we fix its internal content first
                if f.lower() == "meta.cpp":
                    fix_meta_cpp(full_path)
                
                os.utime(full_path, (now, now))
                count += 1
            except:
                pass
    
    print(f"Updated {count} entries.")

if __name__ == "__main__":
    target = ".hemttout"
    if len(sys.argv) > 1:
        target = sys.argv[1]
    
    if os.path.isfile(target):
        if os.path.basename(target).lower() == "meta.cpp":
            fix_meta_cpp(target)
        now = time.time()
        os.utime(target, (now, now))
        print(f"Updated file timestamp: {target}")
    else:
        fix_timestamps(target)
