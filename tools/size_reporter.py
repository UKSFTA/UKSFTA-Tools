#!/usr/bin/env python3
import os
import sys
import math
from pathlib import Path

def get_size(start_path):
    total_size = 0
    file_sizes = []
    for dirpath, dirnames, filenames in os.walk(start_path):
        for f in filenames:
            fp = os.path.join(dirpath, f)
            if not os.path.islink(fp):
                size = os.path.getsize(fp)
                total_size += size
                file_sizes.append((fp, size))
    return total_size, file_sizes

def format_size(size_bytes):
    if size_bytes == 0: return "0 B"
    size_name = ("B", "KB", "MB", "GB", "TB")
    i = int(math.floor(math.log(size_bytes, 1024)))
    p = math.pow(1024, i)
    s = round(size_bytes / p, 2)
    return "%s %s" % (s, size_name[i])

def main():
    build_dir = Path(".hemttout/build")
    if not build_dir.exists():
        print("No build directory found.")
        return

    print("📊 Mod Size Report")
    print("===================")
    
    total_size, files = get_size(build_dir)
    
    # Group by PBO
    pbos = {}
    other_files = []
    
    for f, s in files:
        if f.endswith(".pbo"):
            pbos[os.path.basename(f)] = s
        else:
            other_files.append((f, s))
            
    print(f"\n📦 Total Size: {format_size(total_size)}")
    print(f"🧩 PBO Count:  {len(pbos)}")
    
    print("\nLargest Components:")
    sorted_pbos = sorted(pbos.items(), key=lambda item: item[1], reverse=True)
    for name, size in sorted_pbos[:10]:
        print(f"  - {name:<30} {format_size(size)}")
        
    # Generate GitHub Step Summary if running in CI
    if "GITHUB_STEP_SUMMARY" in os.environ:
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(f"### 📊 Mod Size Report\n\n")
            f.write(f"**Total Size:** `{format_size(total_size)}`  \n")
            f.write(f"**PBO Count:** `{len(pbos)}`  \n\n")
            f.write("| PBO Name | Size |\n")
            f.write("| :--- | :---: |\n")
            for name, size in sorted_pbos:
                f.write(f"| {name} | {format_size(size)} |\n")

if __name__ == "__main__":
    main()
