#!/usr/bin/env python3
import os
import re
import sys
from pathlib import Path

def fix_markdown(file_path):
    path = Path(file_path)
    if not path.exists(): return
    
    content = path.read_text()
    
    # MD022: Blanks around headings
    # Ensure blank line before headings (except at start of file)
    content = re.sub(r'([^
])
(#{1,6}\s)', r'\1

\2', content)
    # Ensure blank line after headings
    content = re.sub(r'(#{1,6}\s.*)
([^
])', r'\1

\2', content)
    
    # MD031: Blanks around fences
    content = re.sub(r'([^
])
(```)', r'\1

\2', content)
    content = re.sub(r'(```)
([^
])', r'\1

\2', content)
    
    # MD032: Blanks around lists
    # This is tricky with regex, but let's handle simple cases
    # Non-list line followed by list item
    content = re.sub(r'([^
\-\*\+\d])
([\-\*\+]|\d+\.)\s', r'\1

\2 ', content)
    # List item followed by non-list line
    content = re.sub(r'(
[\-\*\+]|\d+\.\s.*)
([^
\-\*\+\d])', r'\1

\2', content)

    # MD009: Trailing spaces
    content = re.sub(r'[ 	]+$', '', content, flags=re.MULTILINE)
    
    # MD026: No trailing punctuation in heading
    content = re.sub(r'(#{1,6}\s+.*?)[:\.,;!]\s*$', r'\1', content, flags=re.MULTILINE)

    path.write_text(content)
    print(f"  Fixed: {file_path}")

def main():
    target = sys.argv[1] if len(sys.argv) > 1 else "."
    for root, _, files in os.walk(target):
        for f in files:
            if f.endswith(".md"):
                fix_markdown(os.path.join(root, f))

if __name__ == "__main__":
    main()
