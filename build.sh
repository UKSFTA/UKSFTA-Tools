#!/bin/bash

# UKSFTA Universal HEMTT Wrapper
# Ensures all builds have corrected timestamps and artifacts are archived.

echo "UKSF Taskforce Alpha - HEMTT Wrapper"
echo "=========================================="

# Export current time for deterministic build tools
export CURRENT_UNIX_TIME=$(date +%s)
export SOURCE_DATE_EPOCH=$CURRENT_UNIX_TIME

# Check if 'release' is anywhere in the arguments
IS_RELEASE=false
for arg in "$@"; do
    if [[ "$arg" == "release" ]]; then
        IS_RELEASE=true
        break
    fi
done

if command -v hemtt &> /dev/null; then
    echo "Running: hemtt $@"
    SOURCE_DATE_EPOCH=$CURRENT_UNIX_TIME hemtt "$@"
    BUILD_STATUS=$?
else
    echo "Error: HEMTT not found. Please run .uksf_tools/bootstrap.sh first."
    exit 1
fi

# Fix timestamps and archive if successful
if [ $BUILD_STATUS -eq 0 ]; then
    # Hemtt 'release' prepares the files, 'archive' creates the zip.
    if [ "$IS_RELEASE" = true ]; then
        echo "Release build successful. Packaging ZIP..."
        hemtt archive
    fi

    if [ -f "tools/fix_timestamps.py" ]; then
        echo "Normalizing output timestamps..."
        python3 tools/fix_timestamps.py .hemttout
        if [ -d "releases" ]; then
            python3 tools/fix_timestamps.py releases
        fi
        python3 tools/fix_timestamps.py meta.cpp 2>/dev/null
    fi
    echo "Task complete and timestamps normalized."
else
    echo "Task failed. Skipping timestamp fix."
    exit $BUILD_STATUS
fi
