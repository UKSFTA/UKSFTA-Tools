#!/bin/bash
# ⚔️ UKSFTA Mikero Tools Automated Installer
# Installs extractpbo, derap, and required libraries for Linux.

set -e

MIKERO_VERSION="0.10.13"
TEMP_DIR="/tmp/mikero_install"
# Using the specific verified download link
INSTALL_URL="https://mikero.bytex.digital/api/download?filename=depbo-tools-${MIKERO_VERSION}-linux-amd64.tgz"

echo -e "\033[1;34m"
echo " ⚔️  UKSF TASKFORCE ALPHA | MIKERO TOOLS INSTALLER"
echo " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "\033[0m"

# 1. Prepare Environment
echo "📂 Preparing temporary workspace..."
rm -rf "$TEMP_DIR" && mkdir -p "$TEMP_DIR"
cd "$TEMP_DIR"

# 2. Locate or Download Tools
# Broad search for any existing Mikero assets to avoid blocked downloads
echo "🔍 Searching for local Mikero assets..."
FOUND_PATH=$(find "$HOME" -name "depbo-tools-*" -not -path "*/.local/share/Trash/*" | head -n 1)

if [ -n "$FOUND_PATH" ]; then
    echo "📦 Found local asset: $FOUND_PATH"
    if [ -d "$FOUND_PATH" ]; then
        cp -r "$FOUND_PATH" .
        cd "$(basename "$FOUND_PATH")"
    else
        cp "$FOUND_PATH" mikero_asset.tgz
        tar -xzf mikero_asset.tgz
        cd depbo-tools-*
    fi
else
    echo "📥 Downloading Mikero Tools v${MIKERO_VERSION}..."
    # Using the provided URL with robust headers
    curl -L -H "User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36" \
         -o mikero.tgz "$INSTALL_URL"
    
    # Check if download was successful (not an empty or html error page)
    if [ ! -s mikero.tgz ] || grep -q "<html" mikero.tgz; then
        echo "❌ Error: Download blocked or failed. Please download manually and place in Downloads."
        exit 1
    fi

    echo "📦 Extracting binaries..."
    tar -xzf mikero.tgz
    cd depbo-tools-*
fi

# 4. Install Libraries
echo "🔧 Installing shared libraries (requires sudo)..."
sudo cp -v lib/*.so /usr/local/lib/
sudo ldconfig

# 5. Install Binaries
echo "🚀 Installing command-line tools..."
sudo cp -v bin/* /usr/local/bin/

# 6. Cleanup
echo "🧹 Cleaning up..."
rm -rf "$TEMP_DIR"

echo " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "\033[1;32m🎉 Success! Mikero Tools are now installed.\033[0m"
echo "   Commands available: extractpbo, derap, make pbo, etc."
echo " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
