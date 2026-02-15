#!/usr/bin/env bash
PROJECT_ROOT=$(pwd)
export SOURCE_DATE_EPOCH=$(date +%s)

# Detect if this is a HEMTT project
IS_MOD_PROJECT=false
if [ -d ".hemtt" ] && [ -f ".hemtt/project.toml" ]; then
    IS_MOD_PROJECT=true
fi

# 1. Forensic Audit (UKSFTA Diamond Standard)
# We run this BEFORE hemtt to ensure the build environment is clean.
if [ "$IS_MOD_PROJECT" = true ] && [ -f "tools/asset_auditor.py" ]; then
    echo "🛡️  UKSFTA Forensic Audit: Executing deep-scan..."
    python3 tools/asset_auditor.py .
    AUDIT_STATUS=$?
    
    if [ $AUDIT_STATUS -ne 0 ]; then
        echo "❌ FAIL: Forensic Audit detected critical defects. Building halted."
        exit 1
    fi
    echo "✅ PASS: Asset integrity verified."
fi

# 2. Build Logic
if [ "$IS_MOD_PROJECT" = true ]; then
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
else
    echo "ℹ️  UKSFTA-Tools: Tool-only project detected. Skipping HEMTT."
    STATUS=0
    if [[ " $* " == *" release "* ]]; then
        IS_RELEASE=true
    else
        IS_RELEASE=false
    fi
fi

if [ $STATUS -eq 0 ]; then
    # 3. Fix timestamps (Mod only)
    if [ "$IS_MOD_PROJECT" = true ] && [ -f "tools/fix_timestamps.py" ]; then
        PROJECT_NAME=$(grep 'name =' mod.cpp | head -n 1 | cut -d'"' -f2)
        WORKSHOP_ID=$(grep "workshop_id =" .hemtt/project.toml | head -n 1 | sed -E 's/workshop_id = "(.*)"/\1/' | xargs)
        python3 tools/fix_timestamps.py .hemttout "$PROJECT_NAME" "$WORKSHOP_ID"
    fi

    # 4. Manual Packaging for releases
    if [ "$IS_RELEASE" = true ]; then
        echo "📦 Packaging Release ZIP..."
        mkdir -p releases
        
        VERSION="0.0.0"
        if [ -f "VERSION" ]; then
            VERSION=$(cat VERSION | tr -d '\n\r ')
        fi
        
        PROJECT_ID=$(basename "$PROJECT_ROOT")
        ZIP_NAME="uksf task force alpha - ${PROJECT_ID,,}_${VERSION}.zip"
        STAGING_DIR=".hemttout/zip_staging"
        rm -rf "$STAGING_DIR"
        
        if [ "$IS_MOD_PROJECT" = true ]; then
            # Mod Packaging
            MOD_FOLDER_NAME="@${PROJECT_ID}"
            mkdir -p "$STAGING_DIR/$MOD_FOLDER_NAME"
            cp -rp .hemttout/release/* "$STAGING_DIR/$MOD_FOLDER_NAME/"
            (cd "$STAGING_DIR" && zip -q -1 -r "$PROJECT_ROOT/releases/$ZIP_NAME" "$MOD_FOLDER_NAME")
        else
            # Tool Packaging (Exclude git and build artifacts)
            mkdir -p "$STAGING_DIR/$PROJECT_ID"
            rsync -aq --exclude=".git" --exclude=".hemttout" --exclude="releases" --exclude="all_releases" ./ "$STAGING_DIR/$PROJECT_ID/"
            (cd "$STAGING_DIR" && zip -q -1 -r "$PROJECT_ROOT/releases/$ZIP_NAME" "$PROJECT_ID")
        fi
        
        # Consolidate to Unit Hub
        CENTRAL_HUB=""
        if [ -d "../UKSFTA-Tools/all_releases" ]; then
            CENTRAL_HUB="../UKSFTA-Tools/all_releases"
        fi

        if [ -n "$CENTRAL_HUB" ] && [ "$PROJECT_ID" != "UKSFTA-Tools" ]; then
            echo "  - Consolidating release to Unit Hub..."
            mv "$PROJECT_ROOT/releases/$ZIP_NAME" "$CENTRAL_HUB/"
            echo "✨ Release consolidated to: $CENTRAL_HUB/$ZIP_NAME"
        else
            echo "✨ Release packaged: releases/$ZIP_NAME"
        fi
        
        rm -rf "$STAGING_DIR"
    fi
fi
exit $STATUS
