#!/bin/bash

# UKSFTA Universal HEMTT Wrapper
# Ensures all builds have corrected timestamps and artifacts are archived.

echo "UKSF Taskforce Alpha - HEMTT Wrapper"
echo "=========================================="

# Export current time for deterministic build tools
# Setting this explicitly to 'now' prevents old defaults like 1881
CURRENT_UNIX_TIME=$(date +%s)
export SOURCE_DATE_EPOCH=$CURRENT_UNIX_TIME

# Default to 'build' if no command provided
HEMTT_CMD=${1:-build}
shift # Remove the command from the arguments list

if command -v hemtt &> /dev/null; then
    echo "Running: hemtt $HEMTT_CMD $@"
    # Run hemtt with the current time forced into the environment
    SOURCE_DATE_EPOCH=$CURRENT_UNIX_TIME hemtt "$HEMTT_CMD" "$@"
    BUILD_STATUS=$?
else
    echo "Error: HEMTT not found. Please run .uksf_tools/bootstrap.sh first."
    exit 1
fi

# Fix timestamps and archive if successful
if [ $BUILD_STATUS -eq 0 ]; then
    if [[ "$HEMTT_CMD" == "release" ]]; then
        echo "Release build successful. Packaging ZIP..."
        hemtt archive
    fi

    if [ -f "tools/fix_timestamps.py" ]; then
        echo "Normalizing output timestamps..."
        python3 tools/fix_timestamps.py .hemttout
        if [ -d "releases" ]; then
            python3 tools/fix_timestamps.py releases
        fi
        
        # Explicitly fix meta.cpp timestamp to current time (Arma format)
        # Note: meta.cpp timestamp is technically opaque but we want it fresh
        python3 tools/fix_timestamps.py meta.cpp 2>/dev/null
    fi
    echo "Task complete and timestamps normalized."
else
    echo "Task failed. Skipping timestamp fix."
    exit $BUILD_STATUS
fi
