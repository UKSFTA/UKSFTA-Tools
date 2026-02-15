import os
from pathlib import Path

# Match exactly what hemtt is warning about
OLD = b"z\\uksfta\\addons\\kit\\vests"
NEW = b"z\\uksfta\\addons\\jacks_vests"

def fix_file(path):
    with open(path, 'rb') as f:
        data = f.read()
    if OLD in data:
        print(f"  ✅ Patching: {path}")
        new_data = data.replace(OLD, NEW)
        with open(path, 'wb') as f:
            f.write(new_data)

for root, _, files in os.walk("../UKSFTA-JacksKit-Test"):
    for f in files:
        if f.endswith(('.p3d', '.cpp', '.hpp', '.rvmat')):
            fix_file(os.path.join(root, f))
