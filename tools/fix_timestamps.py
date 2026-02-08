import os
import sys
import time

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
    
    if os.path.isfile(target) and target.endswith(".zip"):
        # If it's a zip, we can't easily fix internal times without re-zipping
        # But we can at least fix the zip file's own time
        now = time.time()
        os.utime(target, (now, now))
        print(f"Updated zip file timestamp: {target}")
    else:
        fix_timestamps(target)
