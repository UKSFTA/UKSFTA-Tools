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
        # Dynamically find the first version file in any addon component
        VERSION_FILE=$(find addons -name "script_version.hpp" | head -n 1)
        if [ -z "$VERSION_FILE" ]; then
            echo "ERROR: Could not find script_version.hpp in any addon folder."
            exit 1
        fi
        
        VERSION=$(grep "#define PATCHLVL" "$VERSION_FILE" | awk '{print $3}' | tr -d '\n\r ')
        MAJOR=$(grep "#define MAJOR" "$VERSION_FILE" | awk '{print $3}' | tr -d '\n\r ')
        MINOR=$(grep "#define MINOR" "$VERSION_FILE" | awk '{print $3}' | tr -d '\n\r ')
        
        # Get Project ID (Folder Name) for unique ZIP/Folder naming
        PROJECT_ID=$(basename "$PROJECT_ROOT")
        MOD_FOLDER_NAME="@${PROJECT_ID}"
        ZIP_NAME="uksf task force alpha - ${PROJECT_ID,,}_${MAJOR}.${MINOR}.${VERSION}.zip"
        
        STAGING_DIR=".hemttout/zip_staging"
        rm -rf "$STAGING_DIR"
        mkdir -p "$STAGING_DIR/$MOD_FOLDER_NAME"
        
        echo "  - Staging release contents..."
        # Use -p to preserve timestamps during copy
        cp -rp .hemttout/release/* "$STAGING_DIR/$MOD_FOLDER_NAME/"
        
        echo "  - Synchronizing staging metadata..."
        # Ensure staging metadata is fixed
        python3 tools/fix_timestamps.py "$STAGING_DIR" "$PROJECT_NAME" "$WORKSHOP_ID"
        
        echo "  - Creating ZIP archive (Fast Mode): $ZIP_NAME"
        # Use -1 for fastest compression to speed up the process
        (cd "$STAGING_DIR" && zip -q -1 -r "$PROJECT_ROOT/releases/$ZIP_NAME" "$MOD_FOLDER_NAME")
        
        # Centralization support: Detect the Unit Hub
        # 1. Check parent directory (Standalone)
        # 2. Check local submodule directory (.uksf_tools)
        CENTRAL_HUB=""
        if [ -d "../UKSFTA-Tools/all_releases" ]; then
            CENTRAL_HUB="../UKSFTA-Tools/all_releases"
        elif [ -d ".uksf_tools/all_releases" ]; then
            CENTRAL_HUB=".uksf_tools/all_releases"
        fi

        if [ -n "$CENTRAL_HUB" ]; then
            echo "  - Consolidating release to Unit Hub..."
            mv "$PROJECT_ROOT/releases/$ZIP_NAME" "$CENTRAL_HUB/"
            rmdir "$PROJECT_ROOT/releases" 2>/dev/null
            echo "✨ Release consolidated to: $CENTRAL_HUB/$ZIP_NAME"
        else
            echo "📦 Release packaged: releases/$ZIP_NAME"
        fi
        
        rm -rf "$STAGING_DIR"
    fi
fi
exit $STATUS
