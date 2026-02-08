#!/bin/bash

# UKSFTA Release Shortcut
# Ensures release builds have corrected timestamps and archives are generated.

# We run hemtt release directly and then fix timestamps
echo "UKSF Taskforce Alpha - HEMTT Release"
echo "=========================================="

export SOURCE_DATE_EPOCH=$(date +s)

if command -v hemtt &> /dev/null; then
    hemtt release "$@"
    BUILD_STATUS=$?
else
    echo "Error: HEMTT not found."
    exit 1
fi

if [ $BUILD_STATUS -eq 0 ]; then
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
        [ -d "releases" ] && python3 tools/fix_timestamps.py releases
    fi
    echo "Release complete."
else
    exit $BUILD_STATUS
fi
