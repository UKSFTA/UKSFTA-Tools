#!/bin/bash

# UKSFTA New Project Scaffold
# Usage: ./new_project.sh [PROJECT_NAME] [TEMPLATE: standard|cba|mission|ui]

if [ -z "$1" ]; then
    echo "Usage: ./new_project.sh [PROJECT_NAME] [TEMPLATE: standard|cba|mission|ui]"
    exit 1
fi

PROJECT_NAME=$1
TEMPLATE=${2:-standard}
TOOLS_DIR=$(pwd)
PROJECT_DIR="../$PROJECT_NAME"

if [[ "$TEMPLATE" != "standard" && "$TEMPLATE" != "cba" && "$TEMPLATE" != "mission" && "$TEMPLATE" != "ui" ]]; then
    echo "Error: Template must be 'standard' or 'cba'."
    exit 1
fi

echo "Creating new UKSFTA project: $PROJECT_NAME (Template: $TEMPLATE)"

# 1. Scaffold from Template
if [ -d "$PROJECT_DIR" ]; then
    echo "Error: Directory $PROJECT_DIR already exists."
    exit 1
fi

mkdir -p "$PROJECT_DIR"
cp -r templates/"$TEMPLATE"/* "$PROJECT_DIR/"
cd "$PROJECT_DIR"

# 2. Initialize Git
git init
git branch -M main

# 3. Setup Submodule (using the current tools repo as source)
git submodule add git@github.com:UKSFTA/UKSFTA-Tools.git .uksf_tools
python3 .uksf_tools/setup.py

# 4. Customize metadata
echo "1.0.0" > VERSION
sed -i "s/Project/$PROJECT_NAME/g" mod.cpp
sed -i "s/Project/$PROJECT_NAME/g" .hemtt/project.toml
sed -i "s/Project/$PROJECT_NAME/g" addons/main/script_version.hpp 2>/dev/null
sed -i "s/project/${PROJECT_NAME,,}/g" addons/main/\$PBOPREFIX\$ 2>/dev/null
sed -i "s/project/${PROJECT_NAME,,}/g" addons/main/config.cpp 2>/dev/null
if [ "$TEMPLATE" == "cba" ]; then
    sed -i "s/project/${PROJECT_NAME,,}/g" addons/main/script_macros.hpp
fi

# 5. Initial Commit
git add .
git commit -S -m "Initial commit: Scaffolded from UKSFTA-Tools ($TEMPLATE template)"

echo ""
echo "Project $PROJECT_NAME ($TEMPLATE) created successfully at $PROJECT_DIR"
echo "Don't forget to update the workshop_id in .hemtt/project.toml!"
