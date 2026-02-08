#!/bin/bash

# UKSFTA HEMTT Wrapper
# This script handles building, timestamp fixing, and manual archiving.

export SOURCE_DATE_EPOCH=$(date +%s)

# 1. Run HEMTT command
# We use --no-archive to prevent HEMTT from creating empty zips
if [[ " $* " == *"release"* ]]; then
    echo "HEMTT: Running release (no-archive)..."
    hemtt release --no-archive "$@"
    STATUS=$?
else
    echo "HEMTT: Running '$@'..."
    hemtt "$@"
    STATUS=$?
fi

if [ $STATUS -eq 0 ]; then
    # 2. Fix timestamps in .hemttout
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
    fi

    # 3. Manual Archiving for releases
    if [[ " $* " == *"release"* ]]; then
        echo "HEMTT: Manually packaging release ZIP..."
        mkdir -p releases
        
        PROJECT_PREFIX=$(grep "prefix =" .hemtt/project.toml | cut -d'"' -f2)
        MAJOR=$(grep "#define MAJOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
        MINOR=$(grep "#define MINOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
        PATCH=$(grep "#define PATCHLVL" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
        
        ZIP_NAME="uksf task force alpha - ${PROJECT_PREFIX,,}_${MAJOR}.${MINOR}.${PATCH}.zip"
        LATEST_ZIP="${PROJECT_PREFIX}-latest.zip"
        
        # Package the contents of .hemttout/release/
        # Use a subshell to avoid directory issues
        (
            cd .hemttout/release/
            zip -r "../../releases/$ZIP_NAME" ./*
        )
        
        cp "releases/$ZIP_NAME" "releases/$LATEST_ZIP"
        
        # Final timestamp fix on the new ZIPs
        python3 tools/fix_timestamps.py releases
        echo "Release packaged successfully: releases/$ZIP_NAME"
    fi
fi

exit $STATUS
