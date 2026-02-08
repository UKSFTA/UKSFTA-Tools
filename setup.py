import os
import shutil
import sys

def setup_project():
    project_root = os.getcwd()
    tools_dir = os.path.dirname(os.path.abspath(__file__))
    
    print(f"Setting up UKSFTA Tools in: {project_root}")
    
    # 1. Create directory structure
    dirs = [
        "tools",
        ".hemtt/scripts",
        ".hemtt/hooks",
        ".github/workflows"
    ]
    for d in dirs:
        os.makedirs(os.path.join(project_root, d), exist_ok=True)

    # 2. Copy Tools
    # We copy the python tools so they are real files for CI and ease of use
    python_tools_src = os.path.join(tools_dir, "tools")
    python_tools_dst = os.path.join(project_root, "tools")
    
    if os.path.exists(python_tools_dst):
        if os.path.islink(python_tools_dst):
            os.remove(python_tools_dst)
        else:
            shutil.rmtree(python_tools_dst)
    
    shutil.copytree(python_tools_src, python_tools_dst)
    print(f" Copied: tools/ directory")

    # 3. Symlink HEMTT Scripts/Hooks
    for category in ["scripts", "hooks"]:
        src_path_abs = os.path.join(tools_dir, "hemtt", category)
        if not os.path.exists(src_path_abs): continue
        
        for item in os.listdir(src_path_abs):
            src_abs = os.path.join(src_path_abs, item)
            dst_abs = os.path.join(project_root, ".hemtt", category, item)
            
            rel_src = os.path.relpath(src_abs, os.path.dirname(dst_abs))
            
            if os.path.exists(dst_abs):
                if os.path.islink(dst_abs) or os.path.isfile(dst_abs):
                    os.remove(dst_abs)
                elif os.path.isdir(dst_abs):
                    shutil.rmtree(dst_abs)
            os.symlink(rel_src, dst_abs)
            print(f" Linked: .hemtt/{category}/{item} -> {rel_src}")

    # 4. Copy GitHub Workflows
    workflow_src_dir = os.path.join(tools_dir, ".github", "workflows")
    if os.path.exists(workflow_src_dir):
        for item in os.listdir(workflow_src_dir):
            src = os.path.join(workflow_src_dir, item)
            dst = os.path.join(project_root, ".github", "workflows", item)
            if os.path.exists(dst):
                if os.path.islink(dst) or os.path.isfile(dst):
                    os.remove(dst)
                elif os.path.isdir(dst):
                    shutil.rmtree(dst)
            shutil.copy2(src, dst)
            print(f" Copied: .github/workflows/{item}")

    # 5. Copy templates if missing
    for template in ["workshop_description.txt", ".env.example"]:
        dst = os.path.join(project_root, template)
        if not os.path.exists(dst):
            shutil.copy(os.path.join(tools_dir, template), dst)
            print(f" Copied: {template}")

    # 6. Ensure .gitignore covers releases
    gitignore_path = os.path.join(project_root, ".gitignore")
    ignore_rule = "/releases/*.zip"
    if os.path.exists(gitignore_path):
        with open(gitignore_path, "r") as f:
            content = f.read()
        if ignore_rule not in content:
            with open(gitignore_path, "a") as f:
                f.write(f"\n# Added by UKSFTA Tools\n{ignore_rule}\n")
            print(f" Updated: .gitignore (added {ignore_rule})")
    else:
        template_src = os.path.join(tools_dir, ".gitignore_template")
        if os.path.exists(template_src):
            shutil.copy(template_src, gitignore_path)
            print(f" Created: .gitignore from template")

    print("\nSetup complete! Your project is now linked to UKSFTA-Tools.")

if __name__ == "__main__":
    setup_project()
