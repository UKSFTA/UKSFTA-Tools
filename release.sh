#!/bin/bash

# UKSFTA Release Shortcut
# Ensures release builds have corrected timestamps and archives are generated.

# We call build.sh with the 'release' command
bash build.sh release "$@"
