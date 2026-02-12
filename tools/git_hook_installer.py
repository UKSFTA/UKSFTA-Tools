#!/usr/bin/env python3
import os
import sys
from pathlib import Path

# UKSFTA Git Hook Installer
# Installs a pre-commit hook to run syntax fixes and security audits.

HOOK_CONTENT = """#!/bin/bash
# UKSFTA Pre-Commit Hook
echo "🛡️  UKSFTA Quality Guard: Running pre-commit checks..."

# 1. Fix Syntax (Automated)
python3 .uksf_tools/tools/syntax_fixer.py .

# 2. Audit Security (Leaks)
python3 .uksf_tools/tools/security_auditor.py .
if [ $? -ne 0 ]; then
    echo "❌ SECURITY LEAK DETECTED. Commit aborted."
    exit 1
fi

# 3. Audit Strings
python3 .uksf_tools/tools/string_auditor.py .
if [ $? -ne 0 ]; then
    echo "⚠️  Localization errors found. Please review."
fi

# Re-add fixed files if they were modified by syntax_fixer
git add .

echo "✅ Quality Guard: All checks passed."
exit 0
"""

def install_hooks(project_path):
    root = Path(project_path)
    git_dir = root / ".git"
    
    if not git_dir.exists():
        print(f"Error: {project_path} is not a git repository.")
        return False

    hooks_dir = git_dir / "hooks"
    hooks_dir.mkdir(exist_ok=True)
    
    pre_commit = hooks_dir / "pre-commit"
    with open(pre_commit, "w") as f:
        f.write(HOOK_CONTENT)
    
    os.chmod(pre_commit, 0o755)
    print(f"  ✅ Installed UKSFTA Pre-Commit Hook in: {project_path}")
    return True

if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else "."
    install_hooks(target)
