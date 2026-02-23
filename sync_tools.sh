#!/bin/bash

# UKSFTA Submodule Sync Script
# This script dynamically updates the .uksf_tools submodule and runs the setup script 
# for every project in the current development directory.

# Get the directory where the script is located
BASE_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"

echo "🛡️  UKSFTA DevOps: Synchronizing Unit Infrastructure..."
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Find all directories that contain a .uksf_tools folder
mapfile -t PROJECTS < <(find "$BASE_DIR" -maxdepth 2 -name ".uksf_tools" -type d -exec dirname {} \;)

if [ ${#PROJECTS[@]} -eq 0 ]; then
    echo "❌ No projects with .uksf_tools found."
    exit 1
fi

for PROJECT_PATH in "${PROJECTS[@]}"; do
    PROJECT=$(basename "$PROJECT_PATH")
    
    echo ""
    echo "📦 Updating: $PROJECT"
    echo "────────────────────────────────────────"
    
    cd "$PROJECT_PATH"
    
    # 1. Update the submodule to the latest commit on main
    # Use --remote to fetch the latest from the origin of the submodule
    git submodule update --init --remote --merge 2>/dev/null
    
    # 2. Run the setup script to refresh tools and HEMTT links
    if [ -f ".uksf_tools/setup.py" ]; then
        python3 .uksf_tools/setup.py
    else
        echo "  ⚠️  setup.py not found in .uksf_tools"
    fi
    
    echo "  ✅ Done."
    
    cd "$BASE_DIR"
done

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✨ ALL PROJECTS SYNCHRONIZED"
