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

    # 2. Copy Tools
    python_tools_src = os.path.join(tools_dir, "tools")
    python_tools_dst = os.path.join(project_root, "tools")
    if os.path.exists(python_tools_dst):
        shutil.rmtree(python_tools_dst) if not os.path.islink(python_tools_dst) else os.remove(python_tools_dst)
    shutil.copytree(python_tools_src, python_tools_dst)
    print(f" Copied: tools/ directory")

    # 3. Symlink HEMTT Scripts/Hooks (Relative symlinks)
    hemtt_src_dir = os.path.join(tools_dir, "hemtt")
    if os.path.exists(hemtt_src_dir):
        for category in os.listdir(hemtt_src_dir):
            src_cat_abs = os.path.join(hemtt_src_dir, category)
            if not os.path.isdir(src_cat_abs): continue
            
            dst_cat_abs = os.path.join(project_root, ".hemtt", category)
            if os.path.exists(dst_cat_abs):
                shutil.rmtree(dst_cat_abs)
            os.makedirs(dst_cat_abs, exist_ok=True)

            for item in os.listdir(src_cat_abs):
                src_item_abs = os.path.join(src_cat_abs, item)
                dst_item_abs = os.path.join(dst_cat_abs, item)
                
                if os.path.isdir(src_item_abs):
                    os.makedirs(dst_item_abs, exist_ok=True)
                    for subitem in os.listdir(src_item_abs):
                        s_abs = os.path.join(src_item_abs, subitem)
                        d_abs = os.path.join(dst_item_abs, subitem)
                        rel_path = os.path.relpath(s_abs, os.path.dirname(d_abs))
                        os.symlink(rel_path, d_abs)
                        print(f" Linked: .hemtt/{category}/{item}/{subitem}")
                else:
                    rel_path = os.path.relpath(src_item_abs, os.path.dirname(dst_item_abs))
                    os.symlink(rel_path, dst_item_abs)
                    print(f" Linked: .hemtt/{category}/{item}")

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
    for template in ["workshop_description.txt", ".env.example", "build.sh", "release.sh", "bootstrap.sh"]:
        dst = os.path.join(project_root, template)
        if template in ["build.sh", "release.sh", "bootstrap.sh"]:
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

    gitignore_path = os.path.join(project_root, ".gitignore")
    if os.path.exists(gitignore_path):
        with open(gitignore_path, "r") as f:
            content = f.read()
        if "releases/" not in content:
            with open(gitignore_path, "a") as f:
                f.write("\nreleases/\n")
            print(f" Updated: .gitignore")

    print("\nSetup complete! Project is now production-ready.")

if __name__ == "__main__":
    setup_project()
