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

    # 2. Symlink or Copy Tools
    # ... (previous logic for tools) ...
    # We use symlinks for the python tools so they update automatically
    python_tools = os.listdir(os.path.join(tools_dir, "tools"))
    for tool in python_tools:
        src = os.path.join(tools_dir, "tools", tool)
        dst = os.path.join(project_root, "tools", tool)
        if os.path.exists(dst):
            if os.path.islink(dst) or os.path.isfile(dst):
                os.remove(dst)
            elif os.path.isdir(dst):
                shutil.rmtree(dst)
        os.symlink(src, dst)
        print(f" Linked: tools/{tool}")

    # 3. Symlink HEMTT Scripts/Hooks
    for category in ["scripts", "hooks"]:
        src_path = os.path.join(tools_dir, "hemtt", category)
        if not os.path.exists(src_path): continue
        
        for item in os.listdir(src_path):
            src = os.path.join(src_path, item)
            dst = os.path.join(project_root, ".hemtt", category, item)
            if os.path.exists(dst):
                if os.path.islink(dst) or os.path.isfile(dst):
                    os.remove(dst)
                elif os.path.isdir(dst):
                    shutil.rmtree(dst)
            os.symlink(src, dst)
            print(f" Linked: .hemtt/{category}/{item}")

    # 4. Copy GitHub Workflows
    workflow_src_dir = os.path.join(tools_dir, "github", "workflows")
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
