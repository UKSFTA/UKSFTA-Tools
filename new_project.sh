#!/bin/bash

# UKSFTA New Project Scaffold
# Usage: ./new_project.sh [PROJECT_NAME]

if [ -z "$1" ]; then
    echo "Usage: ./new_project.sh [PROJECT_NAME]"
    exit 1
fi

PROJECT_NAME=$1
PROJECT_DIR="../$PROJECT_NAME"

echo "Creating new UKSFTA project: $PROJECT_NAME"

# 1. Clone Template
if [ -d "$PROJECT_DIR" ]; then
    echo "Error: Directory $PROJECT_DIR already exists."
    exit 1
fi

# We assume UKSFTA-Template is in the parent directory
if [ ! -d "../UKSFTA-Template" ]; then
    echo "Error: UKSFTA-Template not found in parent directory."
    exit 1
fi

cp -r ../UKSFTA-Template "$PROJECT_DIR"
cd "$PROJECT_DIR"

# 2. Re-initialize Git
rm -rf .git
git init
git branch -M main

# 3. Setup Submodule
git submodule add git@github.com:UKSFTA/UKSFTA-Tools.git .uksf_tools
python3 .uksf_tools/setup.py

# 4. Customize project.toml
sed -i "s/UKSF Task Force Alpha - Template/UKSF Task Force Alpha - $PROJECT_NAME/" .hemtt/project.toml
sed -i "s/UKSFTA-Template/$PROJECT_NAME/" .hemtt/project.toml

# 5. Initial Commit
git add .
git commit -S -m "Initial commit: Scaffolded from UKSFTA-Template"

echo ""
echo "Project $PROJECT_NAME created successfully at $PROJECT_DIR"
echo "Don't forget to update the workshop_id in .hemtt/project.toml!"
