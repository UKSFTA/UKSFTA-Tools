#!/bin/bash

# UKSFTA HEMTT Wrapper
# This script ensures all build artifacts have corrected timestamps and reliable packaging.

export SOURCE_DATE_EPOCH=$(date +%s)

# 1. Run HEMTT command
echo "HEMTT: Executing '$@'..."
hemtt "$@"
STATUS=$?

if [ $STATUS -eq 0 ]; then
    # Give HEMTT a moment to close file handles
    sleep 2

    # 2. Fix timestamps in .hemttout
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
    fi

    # 3. If it was a release, handle final packaging and timestamps
    if [[ " $* " == *"release"* ]]; then
        echo "HEMTT: Finalizing release archives..."
        
        # Normalize existing zip timestamps
        if [ -d "releases" ]; then
            python3 tools/fix_timestamps.py releases
            
            # Manual Renaming (Standardizing unit names)
            PROJECT_PREFIX=$(grep "prefix =" .hemtt/project.toml | cut -d'"' -f2)
            PROJECT_VERSION=$(grep "#define PATCHLVL" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
            MAJOR=$(grep "#define MAJOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
            MINOR=$(grep "#define MINOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
            
            # Find the long-named zip and rename it if needed
            # (Matches HEMTT default: PREFIX-MAJOR.MINOR.PATCH.BUILD.zip)
            find releases -name "${PROJECT_PREFIX}*.zip" -not -name "*-latest.zip" | while read -r zip; do
                NEW_NAME="uksf task force alpha - ${PROJECT_PREFIX,,}_${MAJOR}.${MINOR}.${PROJECT_VERSION}.zip"
                mv "$zip" "releases/$NEW_NAME" 2>/dev/null
                echo "Renamed $zip to $NEW_NAME"
            done
        fi
    fi
fi

exit $STATUS
