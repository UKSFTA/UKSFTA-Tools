#!/bin/bash
PROJECT_ROOT=$(pwd)
export SOURCE_DATE_EPOCH=$(date +%s)

# Extract Project Name from mod.cpp for meta.cpp synchronization
PROJECT_NAME=$(grep 'name =' mod.cpp | head -n 1 | cut -d'"' -f2)

# 1. Run HEMTT
# We add --no-sign as the default unit standard
if [[ " $* " == *" release "* ]]; then
    CLEAN_ARGS=$(echo "$@" | sed 's/release//g')
    echo "HEMTT: Running unsigned release (no-archive) $CLEAN_ARGS..."
    hemtt release --no-archive --no-sign $CLEAN_ARGS
    IS_RELEASE=true
else
    echo "HEMTT: Running '$@'..."
    hemtt "$@"
    IS_RELEASE=false
fi
STATUS=$?

if [ $STATUS -eq 0 ]; then
    # 2. Fix timestamps and Sync meta.cpp name/ID
    if [ -f "tools/fix_timestamps.py" ]; then
        WORKSHOP_ID=$(grep "workshop_id =" .hemtt/project.toml | head -n 1 | sed -E 's/workshop_id = "(.*)"/\1/' | xargs)
        python3 tools/fix_timestamps.py .hemttout "$PROJECT_NAME" "$WORKSHOP_ID"
    fi

    # 3. Manual Packaging for releases
    if [ "$IS_RELEASE" = true ]; then
        echo "HEMTT: Manually packaging ZIP..."
        mkdir -p releases
        PREFIX=$(grep "prefix =" .hemtt/project.toml | head -n 1 | sed -E 's/prefix = "(.*)"/\1/' | xargs)
        VERSION=$(grep "#define PATCHLVL" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        MAJOR=$(grep "#define MAJOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        MINOR=$(grep "#define MINOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        
        MOD_FOLDER_NAME="@${PREFIX}"
        ZIP_NAME="uksf task force alpha - ${PREFIX,,}_${MAJOR}.${MINOR}.${VERSION}.zip"
        
        STAGING_DIR=".hemttout/zip_staging"
        rm -rf "$STAGING_DIR"
        mkdir -p "$STAGING_DIR/$MOD_FOLDER_NAME"
        
        # Copy release contents into the @Folder
        cp -r .hemttout/release/* "$STAGING_DIR/$MOD_FOLDER_NAME/"
        
        # Ensure staging metadata is also fixed and name is synced
        python3 tools/fix_timestamps.py "$STAGING_DIR" "$PROJECT_NAME" "$WORKSHOP_ID"
        
        (cd "$STAGING_DIR" && zip -q -r "$PROJECT_ROOT/releases/$ZIP_NAME" "$MOD_FOLDER_NAME")
        cp "releases/$ZIP_NAME" "releases/${PREFIX}-latest.zip"
        python3 tools/fix_timestamps.py releases
        echo "Release packaged: releases/$ZIP_NAME"
        rm -rf "$STAGING_DIR"
    fi
fi
exit $STATUS
