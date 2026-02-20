#!/usr/bin/env bash
# UKSFTA - Hemtt Build Script
# This script orchestrates the HEMTT build process for Arma 3 mods.
# It handles different build types (dev, release, fast), binarization, and automated packaging.
# ---------------------------------------------------------------------------------

set -e # Exit immediately if a command exits with a non-zero status

# --- CONFIGURATION ---
PROJECT_ROOT=$(pwd)
PROJECT_ID=$(basename "$PROJECT_ROOT")
# HEMTT_OUT is where HEMTT places its build artifacts (e.g., .hemttout/build, .hemttout/release)
HEMTT_OUT="${PROJECT_ROOT}/.hemttout"

# Ensure HEMTT_TEMP_DIR and standard TEMP variables are set to a project-local directory for build stability
export HEMTT_TEMP_DIR="${HEMTT_OUT}/tmp"
export TMPDIR="${HEMTT_TEMP_DIR}"
export TEMP="${HEMTT_TEMP_DIR}"
export TMP="${HEMTT_TEMP_DIR}"
mkdir -p "${HEMTT_TEMP_DIR}"

# Attempt to find the best mod staging directory (for Dev VFS)
find_mod_root() {
    local target_name="$1"
    local sibling
    sibling=$(realpath "${PROJECT_ROOT}/../${target_name}" 2>/dev/null || true)
    if [ -n "$sibling" ] && [ -d "$sibling" ]; then
        echo "$sibling"
        return
    fi
    local parent2
    parent2=$(realpath "${PROJECT_ROOT}/../../${target_name}" 2>/dev/null || true)
    if [ -n "$parent2" ] && [ -d "$parent2" ]; then
        echo "$parent2"
        return
    fi
    realpath "${PROJECT_ROOT}/../${target_name}"
}

ARMA_MOD_ROOT=$(find_mod_root "@${PROJECT_ID}-Dev")

# --- FUNCTIONS ---

log() {
  echo ">>> [UKSFTA Build] $1"
}

run_hemtt() {
  log "Running HEMTT $1..."
  hemtt "$@"
}

error_exit() {
  log "ERROR: $1" >&2
  exit 1
}

# Ensure HEMTT is installed
command -v hemtt >/dev/null 2>&1 || error_exit "HEMTT not found. Please install it."

# Detect if this is a HEMTT project
IS_MOD_PROJECT=false
if [ -d ".hemtt" ] && [ -f ".hemtt/project.toml" ]; then
    IS_MOD_PROJECT=true
fi

# --- MAIN LOGIC ---

# Clean old build artifacts
log "Cleaning old build artifacts..."
rm -rf "${HEMTT_OUT}/build" "${HEMTT_OUT}/release" "${HEMTT_OUT}/zip_staging" || true

# Handle build types
BUILD_TYPE=${1:-dev}
SHIFT_ARGS="${@:2}"

# Calculate default threads (total - 2) if not explicitly provided
THREADS_FLAG=""
if [[ ! "$*" =~ "--threads" ]] && [[ ! "$*" =~ " -t " ]]; then
    TOTAL_THREADS=$(nproc)
    THREADS=$((TOTAL_THREADS > 2 ? TOTAL_THREADS - 2 : 1))
    THREADS_FLAG="--threads $THREADS"
fi

IS_RELEASE=false

case "$BUILD_TYPE" in
  dev)
    log "Performing Development VFS Build (Fastest)..."
    run_hemtt dev $THREADS_FLAG $SHIFT_ARGS
    ;;
  fast)
    log "Performing Fast Solid Build (No Binarization)..."
    run_hemtt build --no-bin $THREADS_FLAG $SHIFT_ARGS
    ;;
  build)
    log "Performing Standard Build (Binarized)..."
    run_hemtt build $THREADS_FLAG $SHIFT_ARGS
    ;;
  release)
    log "Performing Production Release Build (Binarized)..."
    if [ "$IS_MOD_PROJECT" = true ]; then
        # For mods, we use release --no-archive because we handle ZIP packaging manually below
        run_hemtt release --no-archive --no-sign $THREADS_FLAG $SHIFT_ARGS
    else
        run_hemtt release $THREADS_FLAG $SHIFT_ARGS
    fi
    IS_RELEASE=true
    ;;
  *)
    # Fallback for raw commands
    run_hemtt "$@"
    ;;
esac

STATUS=$?

if [ $STATUS -eq 0 ]; then
    # 2. Fix timestamps (Mod only)
    if [ "$IS_MOD_PROJECT" = true ] && [ -f "tools/fix_timestamps.py" ]; then
        log "Applying UKSFTA Forensic Timestamps..."
        P_NAME=$(grep 'name =' mod.cpp | head -n 1 | cut -d'"' -f2 || echo "$PROJECT_ID")
        WORKSHOP_ID=$(grep "workshop_id =" .hemtt/project.toml | head -n 1 | sed -E 's/workshop_id = "(.*)"/\1/' | xargs)
        python3 tools/fix_timestamps.py .hemttout "$P_NAME" "$WORKSHOP_ID"
    fi

    # 3. Manual Packaging for releases
    if [ "$IS_RELEASE" = true ]; then
        log "📦 Packaging UKSFTA Diamond Tier Release..."
        mkdir -p releases
        
        VERSION="0.0.0"
        if [ -f "VERSION" ]; then
            VERSION=$(cat VERSION | tr -d '\n\r ')
        fi
        
        ZIP_NAME="uksf task force alpha - ${PROJECT_ID,,}_${VERSION}.zip"
        STAGING_DIR="${HEMTT_OUT}/zip_staging"
        rm -rf "$STAGING_DIR"
        
        if [ "$IS_MOD_PROJECT" = true ]; then
            # Mod Packaging: Create the @ModName folder structure
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
            log "Consolidating release to Unit Hub..."
            cp "$PROJECT_ROOT/releases/$ZIP_NAME" "$CENTRAL_HUB/"
            log "✨ Release consolidated to: $CENTRAL_HUB/$ZIP_NAME"
        else
            log "✨ Release packaged: releases/$ZIP_NAME"
        fi
    fi
fi

# 4. Create VFS Symlink (for Dev)
if [ "$BUILD_TYPE" == "dev" ]; then
    log "Creating/Updating base VFS symlink..."
    find_vfs_root() {
        if [ -d "$(realpath "${PROJECT_ROOT}/.." 2>/dev/null || true)" ]; then
            echo "$(realpath "${PROJECT_ROOT}/.." 2>/dev/null)/z/uksfta"
            return
        fi
        echo "$(realpath "${PROJECT_ROOT}/../../.." 2>/dev/null)/z/uksfta"
    }
    BASE_SYMLINK_PATH=$(find_vfs_root)
    if [ ! -L "${BASE_SYMLINK_PATH}" ]; then
        mkdir -p "${PROJECT_ROOT}/z/uksfta"
        mkdir -p "$(dirname "${BASE_SYMLINK_PATH}")"
        ln -sfn "${PROJECT_ROOT}/z/uksfta" "${BASE_SYMLINK_PATH}" || log "Warning: Failed to create VFS symlink."
    fi
fi

exit $STATUS
