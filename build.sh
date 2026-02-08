#!/bin/bash

# UKSFTA Universal HEMTT Wrapper
# Ensures all builds have corrected timestamps and artifacts are archived.

echo "UKSF Taskforce Alpha - HEMTT Wrapper"
echo "=========================================="

# Default to 'build' if no command provided
HEMTT_CMD=${1:-build}
shift # Remove the command from the arguments list

if command -v hemtt &> /dev/null; then
    echo "Running: hemtt $HEMTT_CMD $@"
    hemtt "$HEMTT_CMD" "$@"
    BUILD_STATUS=$?
else
    echo "Error: HEMTT not found. Please run .uksf_tools/bootstrap.sh first."
    exit 1
fi

# Fix timestamps and archive if successful
if [ $BUILD_STATUS -eq 0 ]; then
    # Hemtt 'release' prepares the files, 'archive' creates the zip.
    if [[ "$HEMTT_CMD" == "release" ]]; then
        echo "Release build successful. Packaging ZIP..."
        hemtt archive
    fi

    if [ -f "tools/fix_timestamps.py" ]; then
        echo "Normalizing output timestamps..."
        python3 tools/fix_timestamps.py .hemttout
        # Also fix timestamps in releases/ if it exists (for the ZIPs)
        if [ -d "releases" ]; then
            python3 tools/fix_timestamps.py releases
        fi
    fi
    echo "Task complete and timestamps normalized."
else
    echo "Task failed. Skipping timestamp fix."
    exit $BUILD_STATUS
fi
