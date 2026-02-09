#!/usr/bin/env python3
import os
import sys
import re
from pathlib import Path

# High-fidelity patterns for secret detection
PATTERNS = {
    "Discord Webhook": r"https://discord\.com/api/webhooks/\d+/[A-Za-z0-9_-]+",
    "GitHub Token": r"gh[p|o|u|s|r]_[A-Za-z0-9_]{36,255}",
    "Private Key": r"-----BEGIN [A-Z ]+ PRIVATE KEY-----",
    "Generic Token": r"(?i)(api[_-]key|secret|password|token)\s*[:=]\s*['"][A-Za-z0-9_-]{16,}['"]"
}

def audit_security(project_path):
    print(f"🛡️  Guardian Security Audit: {os.path.basename(project_path)}")
    leaks = []
    
    # 1. Check for un-ignored .env files
    env_file = Path(project_path) / ".env"
    if env_file.exists():
        # Check if it's ignored
        gitignore = Path(project_path) / ".gitignore"
        is_ignored = False
        if gitignore.exists():
            if ".env" in gitignore.read_text():
                is_ignored = True
        
        if not is_ignored:
            leaks.append(f"CRITICAL: .env file found and NOT ignored in .gitignore")

    # 2. Scan code for hardcoded secrets
    code_exts = {".cpp", ".hpp", ".sqf", ".py", ".sh", ".yml", ".json", ".xml"}
    for root, _, files in os.walk(project_path):
        if ".git" in root or ".hemttout" in root: continue
        for f in files:
            file_path = Path(root) / f
            if file_path.suffix.lower() in code_exts:
                try:
                    content = file_path.read_text(errors='ignore')
                    for label, pattern in PATTERNS.items():
                        if re.search(pattern, content):
                            leaks.append(f"LEAK: {label} detected in {file_path.relative_to(project_path)}")
                except: pass

    if leaks:
        print(f"  ❌ Security risks identified!")
        for l in leaks:
            print(f"     - {l}")
        return False
    else:
        print("  ✅ No secrets detected. Repository is clean.")
        return True

if __name__ == "__main__":
    if len(sys.argv) < 2:
        print("Usage: security_auditor.py <project_path>")
        sys.exit(1)
    audit_security(sys.argv[1])
