#!/bin/bash

# UKSFTA HEMTT Wrapper
# This script handles building, timestamp fixing, and manual archiving with unit-standard folder structure.

export SOURCE_DATE_EPOCH=$(date +%s)

# 1. Run HEMTT command
echo "HEMTT: Executing '$@'..."
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
    # 2. Fix timestamps in .hemttout immediately
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
    fi

    # 3. Manual Archiving for releases
    if [[ " $* " == *"release"* ]]; then
        echo "HEMTT: Manually packaging unit-standard ZIP..."
        mkdir -p releases
        
        PREFIX=$(grep "prefix =" .hemtt/project.toml | head -n 1 | cut -d'"' -f2 | tr -d '\n\r ')
        MAJOR=$(grep "#define MAJOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        MINOR=$(grep "#define MINOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        PATCH=$(grep "#define PATCHLVL" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        
        MOD_FOLDER_NAME="@${PREFIX}"
        ZIP_NAME="uksf task force alpha - ${PREFIX,,}_${MAJOR}.${MINOR}.${PATCH}.zip"
        LATEST_ZIP="${PREFIX}-latest.zip"
        
        # Prepare staging for ZIP
        STAGING_DIR=".hemttout/zip_staging"
        rm -rf "$STAGING_DIR"
        mkdir -p "$STAGING_DIR/$MOD_FOLDER_NAME"
        
        # Copy release contents into the @Folder
        cp -r .hemttout/release/* "$STAGING_DIR/$MOD_FOLDER_NAME/"
        
        # Normalize timestamps in staging to be extra sure
        python3 tools/fix_timestamps.py "$STAGING_DIR"
        
        # Package from the staging dir so the @Folder is the root of the ZIP
        (
            cd "$STAGING_DIR"
            zip -q -r "../../releases/$ZIP_NAME" "$MOD_FOLDER_NAME"
        )
        
        cp "releases/$ZIP_NAME" "releases/$LATEST_ZIP"
        
        # Final timestamp fix on the new ZIP files
        python3 tools/fix_timestamps.py releases
        echo "Release packaged successfully: releases/$ZIP_NAME"
        rm -rf "$STAGING_DIR"
    fi
fi

exit $STATUS
