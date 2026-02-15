import os
from pathlib import Path

OLD = b"z\\uksfta\\addons\\kit"
NEW = b"z\\jacks_kit\\addons"

def fix_file(path):
    with open(path, 'rb') as f:
        data = f.read()
    if OLD in data:
        print(f"  ✅ Patching: {path}")
        new_data = data.replace(OLD, NEW)
        with open(path, 'wb') as f:
            f.write(new_data)

for root, _, files in os.walk("../UKSFTA-JacksKit"):
    for f in files:
        if f.endswith(('.p3d', '.cpp', '.hpp', '.rvmat')):
            fix_file(os.path.join(root, f))
