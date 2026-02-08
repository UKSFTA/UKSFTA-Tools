#!/bin/bash

# UKSFTA HEMTT Wrapper
# This script ensures all build artifacts have corrected timestamps.

export SOURCE_DATE_EPOCH=$(date +%s)

# 1. Run HEMTT command
echo "HEMTT: Executing '$@'..."
hemtt "$@"
STATUS=$?

if [ $STATUS -eq 0 ]; then
    # Give HEMTT a moment to finalize file writes (especially ZIPs)
    sleep 2

    # 2. Fix timestamps in .hemttout
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
        
        # 3. Fix timestamps in releases folder
        if [ -d "releases" ]; then
            python3 tools/fix_timestamps.py releases
        fi
    fi
fi

exit $STATUS
