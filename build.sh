#!/bin/bash

# UKSFTA Universal HEMTT Wrapper
# This script handles building, timestamp fixing, and manual archiving.

export SOURCE_DATE_EPOCH=$(date +%s)
PROJECT_ROOT=$(pwd)

# 1. Run HEMTT command
# We force --no-archive for releases to use our robust manual zipping
echo "HEMTT: Executing '$@'..."
if [[ " $* " == *"release"* ]]; then
    # Filter out 'release' and run with no-archive
    CLEAN_ARGS=$(echo "$@" | sed 's/release//g')
    hemtt release --no-archive $CLEAN_ARGS
    IS_RELEASE=true
else
    hemtt "$@"
    IS_RELEASE=false
fi
STATUS=$?

if [ $STATUS -eq 0 ]; then
    # 2. Aggressively fix timestamps in .hemttout
    if [ -f "tools/fix_timestamps.py" ]; then
        echo "HEMTT: Normalizing output metadata and timestamps..."
        python3 tools/fix_timestamps.py .hemttout
    fi

    # 3. Handle Packaging for releases
    if [ "$IS_RELEASE" = true ]; then
        echo "HEMTT: Manually packaging unit-standard ZIP archive..."
        mkdir -p releases
        
        PREFIX=$(grep "prefix =" .hemtt/project.toml | head -n 1 | sed -E 's/prefix = "(.*)"/\1/' | xargs)
        MAJOR=$(grep "#define MAJOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        MINOR=$(grep "#define MINOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        PATCH=$(grep "#define PATCHLVL" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        
        MOD_FOLDER_NAME="@${PREFIX}"
        ZIP_NAME="uksf task force alpha - ${PREFIX,,}_${MAJOR}.${MINOR}.${PATCH}.zip"
        
        # We use 'upload_staging' as the common area for transparency
        STAGING_DIR=".hemttout/upload_staging"
        rm -rf "$STAGING_DIR"
        mkdir -p "$STAGING_DIR/$MOD_FOLDER_NAME"
        
        # Copy release contents into the @Folder
        cp -r .hemttout/release/* "$STAGING_DIR/$MOD_FOLDER_NAME/"
        
        # Fix timestamps in staging before zipping
        python3 tools/fix_timestamps.py "$STAGING_DIR"
        
        # Package the @Folder itself into the ZIP
        (
            cd "$STAGING_DIR"
            zip -q -r "$PROJECT_ROOT/releases/$ZIP_NAME" "$MOD_FOLDER_NAME"
        )
        
        cp "releases/$ZIP_NAME" "releases/${PREFIX}-latest.zip"
        
        # Final timestamp fix on the new ZIP files
        python3 tools/fix_timestamps.py releases
        echo "Release packaged successfully: releases/$ZIP_NAME"
    fi
fi

exit $STATUS
