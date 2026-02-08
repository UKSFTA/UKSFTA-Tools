#!/bin/bash
PROJECT_ROOT=$(pwd)
export SOURCE_DATE_EPOCH=$(date +%s)

# 1. Prepare HEMTT Command
# We use a wrapper to ensure we never pass 'release' twice
if [[ " $* " == *" release "* ]]; then
    # We strip 'release' from the arguments to avoid duplication
    # then call hemtt release --no-archive
    CLEAN_ARGS=$(echo "$@" | sed 's/release//g')
    echo "HEMTT: Running release (no-archive) $CLEAN_ARGS..."
    hemtt release --no-archive $CLEAN_ARGS
    IS_RELEASE=true
else
    echo "HEMTT: Running '$@'..."
    hemtt "$@"
    IS_RELEASE=false
fi
STATUS=$?

if [ $STATUS -eq 0 ]; then
    # 2. Fix timestamps in .hemttout immediately
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
    fi

    # 3. Manual Archiving for releases
    if [ "$IS_RELEASE" = true ]; then
        echo "HEMTT: Manually packaging unit-standard ZIP..."
        mkdir -p releases
        
        PREFIX=$(grep "prefix =" .hemtt/project.toml | head -n 1 | cut -d'"' -f2 | tr -d '\n\r ')
        MAJOR=$(grep "#define MAJOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        MINOR=$(grep "#define MINOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        PATCH=$(grep "#define PATCHLVL" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        
        MOD_FOLDER_NAME="@${PREFIX}"
        ZIP_NAME="uksf task force alpha - ${PREFIX,,}_${MAJOR}.${MINOR}.${PATCH}.zip"
        
        # Prepare staging for ZIP
        STAGING_DIR=".hemttout/zip_staging"
        rm -rf "$STAGING_DIR"
        mkdir -p "$STAGING_DIR/$MOD_FOLDER_NAME"
        
        # Copy release contents into the @Folder
        cp -r .hemttout/release/* "$STAGING_DIR/$MOD_FOLDER_NAME/"
        
        # Normalize timestamps in staging
        python3 tools/fix_timestamps.py "$STAGING_DIR"
        
        # Package the @Folder itself into the root of the ZIP
        (
            cd "$STAGING_DIR"
            zip -q -r "$PROJECT_ROOT/releases/$ZIP_NAME" "$MOD_FOLDER_NAME"
        )
        
        cp "releases/$ZIP_NAME" "releases/${PREFIX}-latest.zip"
        python3 tools/fix_timestamps.py releases
        echo "Release packaged: releases/$ZIP_NAME"
        rm -rf "$STAGING_DIR"
    fi
fi

exit $STATUS
