#!/bin/bash

# UKSFTA Automated Build Script
# This script runs HEMTT build and then normalizes file timestamps.

echo "UKSF Taskforce Alpha - Automated Build"
echo "========================================"

# 1. Run HEMTT build
if command -v hemtt &> /dev/null; then
    echo "Running HEMTT build..."
    hemtt build
    BUILD_STATUS=$?
else
    echo "Error: HEMTT not found. Please run .uksf_tools/bootstrap.sh first."
    exit 1
fi

# 2. Fix timestamps if build succeeded
if [ $BUILD_STATUS -eq 0 ]; then
    if [ -f "tools/fix_timestamps.py" ]; then
        echo "Normalizing output timestamps..."
        python3 tools/fix_timestamps.py .hemttout
    fi
    echo "Build complete and timestamps fixed."
else
    echo "Build failed. Skipping timestamp fix."
    exit $BUILD_STATUS
fi
