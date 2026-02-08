import os
import sys
import time
import re

def fix_meta_cpp(directory):
    meta_path = os.path.join(directory, "meta.cpp")
    if not os.path.exists(meta_path):
        return
    
    try:
        # Current time in HEMTT/Arma format (Win32 Ticks approx)
        # We'll just use a fresh large integer if we find one
        with open(meta_path, "r") as f:
            content = f.read()
        
        # Replace timestamp field with a modern value if it looks like an old one
        # HEMTT uses a specific large integer. We can just let it be, 
        # or replace it with current Unix time if needed.
        # However, the filesystem timestamp is what usually triggers the '1881' bug in explorers.
        pass
    except:
        pass

def fix_timestamps(directory):
    if not os.path.exists(directory):
        return
    
    now = time.time()
    print(f"Normalizing timestamps in: {directory} to {time.ctime(now)}")
    
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
            try:
                full_path = os.path.join(root, f)
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
        now = time.time()
        os.utime(target, (now, now))
        print(f"Updated file timestamp: {target}")
    else:
        fix_timestamps(target)
