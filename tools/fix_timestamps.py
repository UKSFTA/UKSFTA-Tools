import os
import sys

def fix_timestamps(directory):
    if not os.path.exists(directory):
        return
    
    print(f"Normalizing timestamps in: {directory}")
    for root, dirs, files in os.walk(directory):
        for d in dirs:
            try:
                os.utime(os.path.join(root, d), None)
            except:
                pass
        for f in files:
            try:
                os.utime(os.path.join(root, f), None)
            except:
                pass

if __name__ == "__main__":
    # Target .hemttout by default
    target = ".hemttout"
    if len(sys.argv) > 1:
        target = sys.argv[1]
    
    fix_timestamps(target)
    print("Timestamps normalized.")
