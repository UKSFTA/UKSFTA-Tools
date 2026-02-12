#!/usr/bin/env python3
import os
import sys
import subprocess
from pathlib import Path

# UKSFTA Git Hook Installer
# Installs local guards for quality, security, and standards.

HOOK_CONTENT = """#!/usr/bin/env bash
# UKSFTA Quality Guard (Pre-Commit)

echo "🛡️  UKSFTA Quality Guard: Executing pre-commit audit..."

# 1. Run full linting suite
./tools/workspace_manager.py lint --fix
LINT_STATUS=$?

if [ $LINT_STATUS -ne 0 ]; then
    echo "❌ FAIL: Linting errors detected. Please fix the issues above before committing."
    exit 1
fi

# 2. Run Master Audit (targeted security & health)
./tools/workspace_manager.py audit-security
SECURITY_STATUS=$?

if [ $SECURITY_STATUS -ne 0 ]; then
    echo "❌ FAIL: Security vulnerabilities detected (leaked tokens/keys). Blocking commit."
    exit 1
fi

echo "✅ PASS: Quality Guard approved. Proceeding with commit."
exit 0
"""

def install_hook(project_path):
    root = Path(project_path)
    git_hooks_dir = root / ".git" / "hooks"
    
    if not git_hooks_dir.exists():
        print(f"Skipping {root.name}: Not a git repository.")
        return

    hook_path = git_hooks_dir / "pre-commit"
    with open(hook_path, "w") as f:
        f.write(HOOK_CONTENT)
    
    os.chmod(hook_path, 0o755)
    print(f"  ✅ Hook Installed: {root.name}")

def main():
    target = sys.argv[1] if len(sys.argv) > 1 else "."
    install_hook(target)

if __name__ == "__main__":
    main()
