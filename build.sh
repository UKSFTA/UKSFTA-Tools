#!/bin/bash

# UKSFTA HEMTT Wrapper
# This script ensures all build artifacts have corrected timestamps.

export SOURCE_DATE_EPOCH=$(date +%s)

# 1. Run HEMTT command
echo "HEMTT: Executing '$@'..."
hemtt "$@"
STATUS=$?

if [ $STATUS -eq 0 ]; then
    # 2. Wait for ZIP files to be populated (async archiving fix)
    if [[ " $* " == *"release"* ]]; then
        echo "HEMTT: Waiting for release archives to finalize..."
        # Check for up to 10 seconds for non-empty zips
        for i in {1..10}; do
            ZIP_SIZE=$(stat -c%s releases/*.zip 2>/dev/null | awk '{s+=$1} END {print s}')
            if [[ -n "$ZIP_SIZE" && "$ZIP_SIZE" -gt 1000 ]]; then
                echo "HEMTT: Archives finalized ($ZIP_SIZE bytes)."
                break
            fi
            sleep 1
        done
    fi

    # 3. Fix timestamps in .hemttout
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
        # 4. Fix timestamps in releases folder
        if [ -d "releases" ]; then
            python3 tools/fix_timestamps.py releases
        fi
    fi
fi

exit $STATUS
