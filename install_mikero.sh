#!/bin/bash
# ⚔️ UKSFTA Mikero Tools Automated Installer
# Installs extractpbo, derap, and required libraries for Linux.

set -e

MIKERO_VERSION="0.10.13"
TEMP_DIR="/tmp/mikero_install"
INSTALL_URL="https://mikero.bytex.digital/api/download?filename=depbo-tools-${MIKERO_VERSION}.tar.bz2"

echo -e "\033[1;34m"
echo " ⚔️  UKSF TASKFORCE ALPHA | MIKERO TOOLS INSTALLER"
echo " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "\033[0m"

# 1. Prepare Environment
echo "📂 Preparing temporary workspace..."
rm -rf "$TEMP_DIR" && mkdir -p "$TEMP_DIR"
cd "$TEMP_DIR"

# 2. Download Tools
echo "📥 Downloading Mikero Tools v${MIKERO_VERSION}..."
# We use a User-Agent to bypass simple anti-leech blocks
curl -L -A "Mozilla/5.0" -o mikero.tar.bz2 "$INSTALL_URL"

# 3. Extract
echo "📦 Extracting binaries..."
tar -xjf mikero.tar.bz2
cd depbo-tools-${MIKERO_VERSION}

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
