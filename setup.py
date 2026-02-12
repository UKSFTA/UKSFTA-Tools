#!/usr/bin/env python3
import os
import shutil
import sys
from pathlib import Path

# UKSFTA Project Setup & Tooling Sync
# Physically copies hooks/scripts for CI compatibility, symlinks tools for development.

class SetupTools:
    def __init__(self, target_dir):
        self.target = Path(target_dir).resolve()
        self.tools_src = Path(__file__).parent / "tools"
        self.hooks_src = Path(__file__).parent / "hemtt" / "hooks"
        self.scripts_src = Path(__file__).parent / "hemtt" / "scripts"
        
        if not self.tools_src.exists():
            print(f"Error: Tools source not found at {self.tools_src}")
            sys.exit(1)

    def setup(self):
        print(f"Setting up UKSFTA Tools in: {self.target}")
        
        # 1. Symlink tools directory (for local dev updates)
        tools_dest = self.target / "tools"
        if tools_dest.exists() or tools_dest.is_symlink():
            if tools_dest.is_symlink():
                os.remove(tools_dest)
            else:
                shutil.rmtree(tools_dest)
        
        # Create relative symlink for better portability
        try:
            os.symlink("../UKSFTA-Tools/tools", tools_dest)
            print("  ✅ Symlinked: tools/ directory -> ../UKSFTA-Tools/tools")
        except:
            # Fallback to copy if symlink fails
            shutil.copytree(self.tools_src, tools_dest)
            print("  ℹ️  Copied: tools/ directory (Symlink failed)")

        # 2. Physically copy HEMTT hooks and scripts (CI cannot resolve symlinks in submodules)
        hemtt_dest = self.target / ".hemtt"
        hemtt_dest.mkdir(exist_ok=True)
        
        for folder, src in [("hooks", self.hooks_src), ("scripts", self.scripts_src)]:
            dest = hemtt_dest / folder
            if dest.exists():
                shutil.rmtree(dest)
            shutil.copytree(src, dest)
            print(f"  ✅ Copied: .hemtt/{folder} directory")

        # 3. Update lint.toml
        lint_src = Path(__file__).parent / "hemtt" / "lint.toml"
        if lint_src.exists():
            shutil.copy2(lint_src, hemtt_dest / "lint.toml")
            print("  ✅ Updated: .hemtt/lint.toml")

        # 4. Copy GitHub Workflows
        workflow_src = Path(__file__).parent / ".github" / "workflows"
        workflow_dest = self.target / ".github" / "workflows"
        workflow_dest.mkdir(parents=True, exist_ok=True)
        if workflow_src.exists():
            for wf in workflow_src.glob("*.yml"):
                shutil.copy2(wf, workflow_dest / wf.name)
                print(f"  ✅ Copied: .github/workflows/{wf.name}")

        # 5. Update core scripts
        for script in ["build.sh", "release.sh", "bootstrap.sh", "install_mikero.sh"]:
            src = Path(__file__).parent / script
            if src.exists():
                shutil.copy2(src, self.target / script)
                os.chmod(self.target / script, 0o755)
                print(f"  ✅ Updated: {script}")

        # 6. Organization Integrity Files
        for doc in ["CODE_OF_CONDUCT.md", "SECURITY.md", "CONTRIBUTORS"]:
            src = Path(__file__).parent / doc
            if src.exists():
                shutil.copy2(src, self.target / doc)
                print(f"  ✅ Updated: {doc}")

        # 7. Enforce Diamond Standard .gitignore
        ignore_src = Path(__file__).parent / ".gitignore_template"
        if ignore_src.exists():
            shutil.copy2(ignore_src, self.target / ".gitignore")
            print("  ✅ Updated: .gitignore (Enforced Diamond Standard)")

        # 8. Sync Unit Keys
        keys_dir = self.target / "keys"
        keys_dir.mkdir(exist_ok=True)
        tools_keys = Path(__file__).parent / "tools" / "keys"
        if tools_keys.exists():
            for key in tools_keys.glob("*.bikey"):
                shutil.copy2(key, keys_dir / key.name)
                print(f"  ✅ Synced Key: {key.name}")

        print("\nSetup complete! Project is now production-ready.")

if __name__ == "__main__":
    setup = SetupTools(".")
    setup.setup()
