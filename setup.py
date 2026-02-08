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
        ".hemtt/scripts",
        ".hemtt/hooks/pre_build",
        ".hemtt/hooks/post_build",
        ".hemtt/hooks/post_release",
        ".github/workflows"
    ]
    for d in dirs:
        d_abs = os.path.join(project_root, d)
        if os.path.islink(d_abs):
            print(f" Removing directory symlink: {d}")
            os.remove(d_abs)
        os.makedirs(d_abs, exist_ok=True)

    # 2. Copy Tools
    python_tools_src = os.path.join(tools_dir, "tools")
    python_tools_dst = os.path.join(project_root, "tools")
    if os.path.exists(python_tools_dst):
        shutil.rmtree(python_tools_dst) if not os.path.islink(python_tools_dst) else os.remove(python_tools_dst)
    shutil.copytree(python_tools_src, python_tools_dst)
    print(f" Copied: tools/ directory")

    # 3. Symlink HEMTT Scripts/Hooks
    hemtt_src_dir = os.path.join(tools_dir, "hemtt")
    if os.path.exists(hemtt_src_dir):
        for category in os.listdir(hemtt_src_dir):
            src_cat_abs = os.path.join(hemtt_src_dir, category)
            if not os.path.isdir(src_cat_abs): continue
            
            for item in os.listdir(src_cat_abs):
                src_item_abs = os.path.join(src_cat_abs, item)
                dst_item_abs = os.path.join(project_root, ".hemtt", category, item)
                
                if os.path.isdir(src_item_abs):
                    # Destination MUST be a real directory
                    if os.path.islink(dst_item_abs):
                        os.remove(dst_item_abs)
                    os.makedirs(dst_item_abs, exist_ok=True)
                    
                    for subitem in os.listdir(src_item_abs):
                        s_abs = os.path.join(src_item_abs, subitem)
                        d_abs = os.path.join(dst_item_abs, subitem)
                        rel_path = os.path.relpath(s_abs, os.path.dirname(d_abs))
                        
                        if os.path.exists(d_abs) or os.path.islink(d_abs):
                            os.remove(d_abs) if not os.path.isdir(d_abs) or os.path.islink(d_abs) else shutil.rmtree(d_abs)
                        os.symlink(rel_path, d_abs)
                        print(f" Linked: .hemtt/{category}/{item}/{subitem}")
                else:
                    rel_path = os.path.relpath(src_item_abs, os.path.dirname(dst_item_abs))
                    if os.path.exists(dst_item_abs) or os.path.islink(dst_item_abs):
                        os.remove(dst_item_abs) if not os.path.isdir(dst_item_abs) or os.path.islink(dst_item_abs) else shutil.rmtree(dst_item_abs)
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
    for template in ["workshop_description.txt", ".env.example", "build.sh", "release.sh"]:
        dst = os.path.join(project_root, template)
        if not os.path.exists(dst):
            shutil.copy(os.path.join(tools_dir, template), dst)
            print(f" Copied: {template}")

    gitignore_path = os.path.join(project_root, ".gitignore")
    ignore_rule = "releases/"
    if os.path.exists(gitignore_path):
        with open(gitignore_path, "r") as f:
            content = f.read()
        if ignore_rule not in content:
            # Also clean up the old specific rule if it exists
            new_content = content.replace("/releases/*.zip", "")
            with open(gitignore_path, "w") as f:
                f.write(new_content.strip() + f"\n\n# Added by UKSFTA Tools\n{ignore_rule}\n")
            print(f" Updated: .gitignore")

    print("\nSetup complete! Project is now production-ready.")

if __name__ == "__main__":
    setup_project()
