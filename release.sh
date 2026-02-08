#!/bin/bash

# UKSFTA Release Shortcut
# Ensures release builds have corrected timestamps and archives are generated.

# We call build.sh and force the 'release' command as the first argument
bash build.sh release "$@"
