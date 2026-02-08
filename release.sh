#!/bin/bash

# UKSFTA Release Shortcut
# Ensures release builds have corrected timestamps and archives are generated.

echo "UKSF Taskforce Alpha - HEMTT Release"
echo "=========================================="

export SOURCE_DATE_EPOCH=$(date +%s)

if command -v hemtt &> /dev/null; then
    echo "Running: hemtt release $@"
    hemtt release "$@"
    BUILD_STATUS=$?
else
    echo "Error: HEMTT not found. Please run .uksf_tools/bootstrap.sh first."
    exit 1
fi

# Fix timestamps if successful
if [ $BUILD_STATUS -eq 0 ]; then
    if [ -f "tools/fix_timestamps.py" ]; then
        echo "Normalizing output timestamps..."
        python3 tools/fix_timestamps.py .hemttout
        if [ -d "releases" ]; then
            python3 tools/fix_timestamps.py releases
        fi
        python3 tools/fix_timestamps.py meta.cpp 2>/dev/null
    fi
    echo "Release complete and timestamps normalized."
else
    echo "Release failed."
    exit $BUILD_STATUS
fi
