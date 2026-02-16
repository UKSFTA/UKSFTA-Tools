#!/usr/bin/env bash
PROJECT_ROOT=$(pwd)
PROJECT_NAME=$(basename "$PROJECT_ROOT")
export SOURCE_DATE_EPOCH=$(date +%s)

# Detect if this is a HEMTT project
IS_MOD_PROJECT=false
if [ -d ".hemtt" ] && [ -f ".hemtt/project.toml" ]; then
    IS_MOD_PROJECT=true
fi

# 1. Professional Production Launch Staging
if [[ " $* " == *" launch "* ]]; then
    ARMA3_PATHS=(
        "/ext/SteamLibrary/steamapps/common/Arma 3"
        "$HOME/.local/share/Steam/steamapps/common/Arma 3"
        "$HOME/.steam/steam/steamapps/common/Arma 3"
    )
    
    ACTIVE_ARMA=""
    for path in "${ARMA3_PATHS[@]}"; do
        if [ -d "$path" ]; then ACTIVE_ARMA="$path"; break; fi
    done

    if [ -n "$ACTIVE_ARMA" ]; then
        # A. Clear VFS (For production launch, we want the PBOs to handle resolution)
        TARGET_VFS="$ACTIVE_ARMA/z/uksfta"
        if [ -L "$TARGET_VFS" ] || [ -e "$TARGET_VFS" ]; then
            echo "🧹 Cleaning VFS dev link: $TARGET_VFS"
            rm -rf "$TARGET_VFS"
        fi

        # B. Solid Mod Staging (@Name) in Arma 3 Root
        MOD_NAME="@${PROJECT_NAME}"
        EXTERNAL_MOD_PATH="$ACTIVE_ARMA/$MOD_NAME"
        
        echo "📦 Physically staging mod in Arma 3: $EXTERNAL_MOD_PATH"
        rm -rf "$EXTERNAL_MOD_PATH"
        mkdir -p "$EXTERNAL_MOD_PATH/addons"
        mkdir -p "$EXTERNAL_MOD_PATH/keys"
        
        # Build the project
        echo "🔨 Building production PBOs..."
        hemtt build
        
        # Copy files physically (No symlinks)
        echo "  - Copying PBOs and Signatures..."
        cp .hemttout/build/addons/*.pbo "$EXTERNAL_MOD_PATH/addons/" 2>/dev/null
        cp .hemttout/build/addons/*.bisign "$EXTERNAL_MOD_PATH/addons/" 2>/dev/null
        
        echo "  - Copying Keys..."
        cp keys/*.bikey "$EXTERNAL_MOD_PATH/keys/" 2>/dev/null
        
        echo "  - Copying Metadata..."
        cp mod.cpp meta.cpp "$EXTERNAL_MOD_PATH/" 2>/dev/null
        cp addons/main/data/*.paa "$EXTERNAL_MOD_PATH/" 2>/dev/null
        
        echo "✨ Solid Staging Complete."
        
        # C. Launch Arma 3
        # We use hemtt launch to trigger the steam/proton interaction,
        # but we point it EXCLUSIVELY to our physical folder.
        echo "🚀 Launching Arma 3 with physical mod: $MOD_NAME"
        hemtt launch -- -mod="$EXTERNAL_MOD_PATH"
        STATUS=$?
        exit $STATUS
    else
        echo "⚠️  Warning: Arma 3 directory not found. Using HEMTT default."
        hemtt launch
        exit $?
    fi
fi

# 2. Forensic Audit (UKSFTA Diamond Standard)
if [ "$IS_MOD_PROJECT" = true ] && [ -f "tools/asset_auditor.py" ]; then
    echo "🛡️  UKSFTA Forensic Audit: Executing deep-scan..."
    python3 tools/asset_auditor.py .
    if [ $? -ne 0 ]; then echo "❌ FAIL: Forensic Audit detected defects."; exit 1; fi
    echo "✅ PASS: Asset integrity verified."
fi

# 3. Standard Build Logic (Non-launch)
if [ "$IS_MOD_PROJECT" = true ]; then
    echo "HEMTT: Running '$@'..."
    hemtt "$@"
    STATUS=$?
else
    echo "ℹ️  UKSFTA-Tools: Tool-only project detected. Skipping HEMTT."
    STATUS=0
fi

# 4. Fix timestamps & Release Packaging
if [ $STATUS -eq 0 ]; then
    if [ "$IS_MOD_PROJECT" = true ] && [ -f "tools/fix_timestamps.py" ]; then
        P_NAME=$(grep 'name =' mod.cpp | head -n 1 | cut -d'"' -f2)
        WORKSHOP_ID=$(grep "workshop_id =" .hemtt/project.toml | head -n 1 | sed -E 's/workshop_id = "(.*)"/\1/' | xargs)
        python3 tools/fix_timestamps.py .hemttout "$P_NAME" "$WORKSHOP_ID"
    fi
fi
exit $STATUS
