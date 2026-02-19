#!/usr/bin/env bash
# UKSFTA - Hemtt Build Script
# This script orchestrates the HEMTT build process for Arma 3 mods.
# It handles different build types (dev, release), copies PBOs, and manages symlinks.
# ---------------------------------------------------------------------------------

set -e # Exit immediately if a command exits with a non-zero status

# --- CONFIGURATION ---
PROJECT_ROOT=$(pwd)
# HEMTT_OUT is where HEMTT places its build artifacts (e.g., .hemttout/build, .hemttout/release)
HEMTT_OUT="${PROJECT_ROOT}/.hemttout"

# Attempt to find the best mod staging directory
find_mod_root() {
    local target_name="$1"
    # Try 1: Sibling directory (most common in UKSFTA dev env)
    local sibling
    sibling=$(realpath "${PROJECT_ROOT}/../${target_name}" 2>/dev/null || true)
    if [ -n "$sibling" ] && [ -d "$sibling" ]; then
        echo "$sibling"
        return
    fi
    # Try 2: Nested 2 levels deep (e.g. workspace/mods/project)
    local parent2
    parent2=$(realpath "${PROJECT_ROOT}/../../${target_name}" 2>/dev/null || true)
    if [ -n "$parent2" ] && [ -d "$parent2" ]; then
        echo "$parent2"
        return
    fi
    # Try 3: Nested 3 levels deep (e.g. workspace/arma3/mods/project)
    local parent3
    parent3=$(realpath "${PROJECT_ROOT}/../../../${target_name}" 2>/dev/null || true)
    if [ -n "$parent3" ] && [ -d "$parent3" ]; then
        echo "$parent3"
        return
    fi
    # Default: Sibling relative (even if not exists yet)
    realpath "${PROJECT_ROOT}/../${target_name}"
}

# ARMA_MOD_ROOT is the root directory where Arma 3 expects mods to be
ARMA_MOD_ROOT=$(find_mod_root "@UKSFTA-Dev")
RELEASE_MOD_ROOT=$(find_mod_root "@UKSFTA-Release")

# Ensure HEMTT_TEMP_DIR and standard TEMP variables are set to a project-local directory for build stability
export HEMTT_TEMP_DIR="${HEMTT_OUT}/tmp"
export TMPDIR="${HEMTT_TEMP_DIR}"
export TEMP="${HEMTT_TEMP_DIR}"
export TMP="${HEMTT_TEMP_DIR}"
mkdir -p "${HEMTT_TEMP_DIR}"


# --- FUNCTIONS ---

# Function to display messages in a consistent format
log() {
  echo ">>> [UKSFTA Build] $1"
}

# Function to run HEMTT with logging
run_hemtt() {
  log "Running HEMTT $1..."
  hemtt "$@"
}

# Function to handle errors
error_exit() {
  log "ERROR: $1" >&2
  exit 1
}

# Ensure HEMTT is installed
command -v hemtt >/dev/null 2>&1 || error_exit "HEMTT not found. Please install it."

# --- MAIN LOGIC ---

# Clean old build artifacts
log "Cleaning old build artifacts..."
rm -rf "${HEMTT_OUT}/build" "${HEMTT_OUT}/release" || true # '|| true' to prevent error if dir doesn't exist

# Handle build types
BUILD_TYPE=${1:-dev} # Default to 'dev' if no argument is provided
SHIFT_ARGS="${@:2}" # Capture any extra arguments (like --just addon_name)

# Calculate default threads (total - 2) if not explicitly provided in arguments
THREADS_FLAG=""
if [[ ! "$*" =~ "--threads" ]] && [[ ! "$*" =~ " -t " ]]; then
    TOTAL_THREADS=$(nproc)
    THREADS=$((TOTAL_THREADS > 2 ? TOTAL_THREADS - 2 : 1))
    THREADS_FLAG="--threads $THREADS"
fi

case "$BUILD_TYPE" in
  dev)
    log "Performing Development VFS Build (Fastest)..."
    run_hemtt dev $THREADS_FLAG $SHIFT_ARGS
    ;;
  fast)
    log "Performing Fast Solid Build (No Binarization)..."
    run_hemtt build --no-bin $THREADS_FLAG $SHIFT_ARGS
    ;;
  release)
    log "Performing Production Release Build (Binarized)..."
    run_hemtt release $THREADS_FLAG $SHIFT_ARGS
    ;;
  build)
    log "Performing Standard Build (Binarized)..."
    run_hemtt build $THREADS_FLAG $SHIFT_ARGS
    ;;
  *)
    error_exit "Invalid build type: $BUILD_TYPE. Use 'dev', 'fast', 'build', or 'release'."
    ;;
esac

# Stage PBOs
log "Staging PBOs to Arma 3 Mod Root..."

PBO_SOURCE_DIR="${HEMTT_OUT}/${BUILD_TYPE}"
PBO_DEST_ROOT=""

if [ "$BUILD_TYPE" == "dev" ]; then
    PBO_DEST_ROOT="${ARMA_MOD_ROOT}"
elif [ "$BUILD_TYPE" == "release" ]; then
    PBO_DEST_ROOT="${RELEASE_MOD_ROOT}"
fi

if [ -z "$PBO_DEST_ROOT" ]; then
    error_exit "PBO_DEST_ROOT is not set for build type: $BUILD_TYPE"
fi

mkdir -p "${PBO_DEST_ROOT}/addons"

# Automated VFS Conflict Resolution
# Ensure target mod folder is clean before staging
log "Cleaning target mod folder: ${PBO_DEST_ROOT}"
rm -rf "${PBO_DEST_ROOT}/addons"/* || true
rm -rf "${PBO_DEST_ROOT}/keys"/* || true

# Copy PBOs
log "Copying PBOs from ${PBO_SOURCE_DIR} to ${PBO_DEST_ROOT}/addons/"
cp -r "${PBO_SOURCE_DIR}/addons/"* "${PBO_DEST_ROOT}/addons/" || error_exit "Failed to copy PBOs."

# Copy Keys (if they exist)
if [ -d "${PBO_SOURCE_DIR}/keys" ]; then
  log "Copying Keys from ${PBO_SOURCE_DIR}/keys to ${PBO_DEST_ROOT}/keys/"
  mkdir -p "${PBO_DEST_ROOT}/keys"
  cp -r "${PBO_SOURCE_DIR}/keys/"* "${PBO_DEST_ROOT}/keys/" || error_exit "Failed to copy keys."
fi

# Create or Update Base Symlink (for VFS Resolution)
log "Creating/Updating base VFS symlink..."

# Attempt to find the best VFS root (where 'z' should live)
find_vfs_root() {
    # Check parent directory (for UKSFTA standard: /ext/Development/)
    if [ -d "$(realpath "${PROJECT_ROOT}/.." 2>/dev/null || true)" ]; then
        echo "$(realpath "${PROJECT_ROOT}/.." 2>/dev/null)/z/uksfta"
        return
    fi
    # Fallback: Nested 3 levels
    echo "$(realpath "${PROJECT_ROOT}/../../.." 2>/dev/null)/z/uksfta"
}

BASE_SYMLINK_PATH=$(find_vfs_root)

# Resolve existing symlink if it points to a different project
if [ -L "${BASE_SYMLINK_PATH}" ]; then
  CURRENT_LINK_TARGET=$(readlink "${BASE_SYMLINK_PATH}")
  if [[ ! "${CURRENT_LINK_TARGET}" == *"${PROJECT_ROOT}"* ]]; then
    log "Resolving VFS conflict: Existing symlink points to another project. Removing..."
    rm "${BASE_SYMLINK_PATH}"
  fi
fi

# Create symlink if it doesn't exist or was removed
if [ ! -L "${BASE_SYMLINK_PATH}" ]; then
  log "Creating new VFS symlink: ${BASE_SYMLINK_PATH} -> ${PROJECT_ROOT}/z/uksfta"
  mkdir -p "${PROJECT_ROOT}/z/uksfta" # Ensure the source directory exists
  mkdir -p "$(dirname "${BASE_SYMLINK_PATH}")" # Ensure the parent of the symlink exists
  ln -sfn "${PROJECT_ROOT}/z/uksfta" "${BASE_SYMLINK_PATH}" || error_exit "Failed to create VFS symlink."
fi

log "Build process completed successfully for type: $BUILD_TYPE"
