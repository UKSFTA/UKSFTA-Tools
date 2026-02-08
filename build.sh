#!/bin/bash
PROJECT_ROOT=$(pwd)
export SOURCE_DATE_EPOCH=$(date +%s)

# Identify if we are doing a release
IS_RELEASE=false
if [[ " $* " == *" release "* ]]; then
    IS_RELEASE=true
fi

if [ "$IS_RELEASE" = true ]; then
    CLEAN_ARGS=$(echo "$@" | sed 's/release//g')
    echo "HEMTT: Running release (no-archive)..."
    hemtt release --no-archive $CLEAN_ARGS
else
    echo "HEMTT: Running '$@'..."
    hemtt "$@"
fi
STATUS=$?

if [ $STATUS -eq 0 ]; then
    # Fix timestamps in .hemttout immediately
    if [ -f "tools/fix_timestamps.py" ]; then
        python3 tools/fix_timestamps.py .hemttout
    fi

    if [ "$IS_RELEASE" = true ]; then
        echo "HEMTT: Manually packaging unit-standard ZIP..."
        mkdir -p releases
        PREFIX=$(grep "prefix =" .hemtt/project.toml | head -n 1 | cut -d'"' -f2 | tr -d '\n\r ')
        MAJOR=$(grep "#define MAJOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        MINOR=$(grep "#define MINOR" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        PATCH=$(grep "#define PATCHLVL" addons/main/script_version.hpp | awk '{print $3}' | tr -d '\n\r ')
        
        ZIP_NAME="uksf task force alpha - ${PREFIX,,}_${MAJOR}.${MINOR}.${PATCH}.zip"
        
        # Explicit ZIP using local paths
        (
            cd .hemttout/release
            zip -q -r "../../releases/$ZIP_NAME" .
        )
        
        cp "releases/$ZIP_NAME" "releases/${PREFIX}-latest.zip"
        python3 tools/fix_timestamps.py releases
        echo "Release packaged: releases/$ZIP_NAME"
    fi
fi
exit $STATUS
