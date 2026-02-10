import os
import shutil
import sys

def setup_project():
    project_root = os.getcwd()
    tools_dir = os.path.dirname(os.path.abspath(__file__))
    
    if os.path.abspath(project_root) == os.path.abspath(tools_dir):
        print("Running from within Tools repository. Skipping setup.")
        return

    print(f"Setting up UKSFTA Tools in: {project_root}")
    
    # 1. Create directory structure (ensure these are REAL directories)
    dirs = [
        "tools",
        "releases",
        "include",
        ".hemtt/scripts",
        ".hemtt/hooks/pre_build",
        ".hemtt/hooks/post_build",
        ".hemtt/hooks/post_release",
        ".github/workflows"
    ]
    for d in dirs:
        d_abs = os.path.join(project_root, d)
        if os.path.islink(d_abs):
            os.remove(d_abs)
        os.makedirs(d_abs, exist_ok=True)

    # 2. Symlink Tools
    submodule_path = os.path.relpath(tools_dir, project_root)
    python_tools_dst = os.path.join(project_root, "tools")
    if os.path.exists(python_tools_dst) or os.path.islink(python_tools_dst):
        if os.path.islink(python_tools_dst):
            os.remove(python_tools_dst)
        else:
            shutil.rmtree(python_tools_dst)
    
    # Target is .uksf_tools/tools
    target = os.path.join(submodule_path, "tools")
    os.symlink(target, python_tools_dst)
    print(f" Symlinked: tools/ directory -> {target}")

    # 3. Copy HEMTT Scripts/Hooks (Copying for CI VFS compatibility)
    hemtt_src_dir = os.path.join(tools_dir, "hemtt")
    if os.path.exists(hemtt_src_dir):
        for category in os.listdir(hemtt_src_dir):
            src_cat_abs = os.path.join(hemtt_src_dir, category)
            if not os.path.isdir(src_cat_abs): continue
            
            dst_cat_abs = os.path.join(project_root, ".hemtt", category)
            if os.path.exists(dst_cat_abs):
                if os.path.islink(dst_cat_abs):
                    os.remove(dst_cat_abs)
                else:
                    shutil.rmtree(dst_cat_abs)
            
            shutil.copytree(src_cat_abs, dst_cat_abs)
            print(f" Copied: .hemtt/{category} directory")

        # Copy lint.toml to .hemtt root if it exists in templates
        lint_src = os.path.join(tools_dir, "templates", "standard", ".hemtt", "lint.toml")
        lint_dst = os.path.join(project_root, ".hemtt", "lint.toml")
        if os.path.exists(lint_src):
            if os.path.exists(lint_dst): os.remove(lint_dst)
            shutil.copy2(lint_src, lint_dst)
            print(f" Updated: .hemtt/lint.toml")

    # 4. Copy GitHub Workflows
    workflow_src_dir = os.path.join(tools_dir, ".github", "workflows")
    if os.path.exists(workflow_src_dir):
        for item in os.listdir(workflow_src_dir):
            src = os.path.join(workflow_src_dir, item)
            dst = os.path.join(project_root, ".github", "workflows", item)
            if os.path.exists(dst) or os.path.islink(dst):
                os.remove(dst) if not os.path.isdir(dst) or os.path.islink(dst) else shutil.rmtree(dst)
            shutil.copy2(src, dst)
            print(f" Copied: .github/workflows/{item}")

    # 5. Templates & Scripts
    for template in ["workshop_description.txt", ".env.example", "build.sh", "release.sh", "bootstrap.sh", "install_mikero.sh"]:
        dst = os.path.join(project_root, template)
        if template in ["build.sh", "release.sh", "bootstrap.sh", "install_mikero.sh"]:
            # Always overwrite core scripts
            if os.path.exists(dst):
                os.remove(dst)
            shutil.copy(os.path.join(tools_dir, template), dst)
            os.chmod(dst, 0o755) # Ensure executable
            print(f" Updated: {template}")
        elif not os.path.exists(dst):
            # Only copy templates if missing
            shutil.copy(os.path.join(tools_dir, template), dst)
            print(f" Copied: {template}")

    # 6. Enforce Standard .gitignore
    gitignore_src = os.path.join(tools_dir, ".gitignore_template")
    gitignore_dst = os.path.join(project_root, ".gitignore")
    if os.path.exists(gitignore_src):
        shutil.copy2(gitignore_src, gitignore_dst)
        print(f" Updated: .gitignore (Enforced Diamond Standard)")

    # 7. Cleanup Git Index (Remove accidental binaries)
    if os.path.exists(os.path.join(project_root, ".git")):
        # We run this to ensure no PBOs or ZIPs are being tracked
        try:
            # Silence output to keep the setup log clean
            subprocess.run("git rm --cached -r *.pbo *.zip .hemttout/ releases/ 2>/dev/null", 
                           shell=True, cwd=project_root)
        except: pass

    print("\nSetup complete! Project is now production-ready.")

if __name__ == "__main__":
    setup_project()
