#!/bin/bash

# UKSFTA HEMTT Wrapper
# This script handles building, timestamp fixing, and custom archiving.

export SOURCE_DATE_EPOCH=$(date +%s)

# 1. Run HEMTT command
echo "HEMTT: Executing '$@'..."
# We run release with --no-archive since HEMTT's internal zipping is failing
if [[ " $* " == *"release"* ]]; then
    hemtt release --no-archive "$@"
else
    hemtt "$@"
fi
STATUS=$?

if [ $STATUS -eq 0 ]; then
    # 2. Fix timestamps in .hemttout
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
    fi

    # 3. Handle Archiving manually if it was a release
    if [[ " $* " == *"release"* ]]; then
        echo "HEMTT: Creating unit-standard ZIP archive..."
        mkdir -p releases
        
        PROJECT_PREFIX=$(grep "prefix =" .hemtt/project.toml | cut -d'"' -f2)
        MAJOR=$(grep "#define MAJOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
        MINOR=$(grep "#define MINOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
        PATCH=$(grep "#define PATCHLVL" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\r')
        
        ZIP_NAME="uksf task force alpha - ${PROJECT_PREFIX,,}_${MAJOR}.${MINOR}.${PATCH}.zip"
        LATEST_ZIP="${PROJECT_PREFIX}-latest.zip"
        
        # Package the contents of .hemttout/release/
        cd .hemttout/release/
        zip -r "../../releases/$ZIP_NAME" ./*
        cd ../..
        
        cp "releases/$ZIP_NAME" "releases/$LATEST_ZIP"
        
        # Final timestamp fix on the new ZIPs
        python3 tools/fix_timestamps.py releases
        echo "Release packaged: releases/$ZIP_NAME"
    fi
fi

exit $STATUS
