#!/bin/bash
# ⚔️ UKSFTA Platinum DevOps Bootstrap Script
# This script establishes a "Diamond Grade" development environment.
# Supported: Arch (pacman), Debian/Ubuntu (apt), Fedora/RHEL (dnf), BSD/Termux (pkg)

set -e

echo -e "\033[1;34m"
echo " ⚔️  UKSF TASKFORCE ALPHA | DEVOPS BOOTSTRAP"
echo " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "\033[0m"

# --- 1. OS DETECTION & SYSTEM PKG INSTALL ---
install_pkg() {
    if command -v pacman &> /dev/null; then
        echo "📦 [pacman] Installing system dependencies..."
        sudo pacman -S --needed --noconfirm git github-cli python-pip ffmpeg zip unzip gnupg
    elif command -v apt-get &> /dev/null; then
        echo "📦 [apt] Installing system dependencies..."
        sudo apt-get update
        sudo apt-get install -y git gh python3-pip ffmpeg zip unzip gnupg
    elif command -v dnf &> /dev/null; then
        echo "📦 [dnf] Installing system dependencies..."
        sudo dnf install -y git gh python3-pip ffmpeg zip unzip gnupg
    elif command -v pkg &> /dev/null; then
        echo "📦 [pkg] Installing system dependencies..."
        sudo pkg install -y git gh python3-pip ffmpeg zip unzip gnupg
    else
        echo "⚠️  Package manager not recognized. Please ensure git, gh, python3, ffmpeg, and zip are installed."
    fi
}

install_pkg

# --- 2. PYTHON ENVIRONMENT ---
echo "🐍 Setting up Python environment..."
# We use --break-system-packages for modern distros where pip is restricted
# or recommend a venv if they prefer. For unit use, direct install is often easier.
python3 -m pip install --upgrade rich --break-system-packages 2>/dev/null || \
python3 -m pip install --upgrade rich || \
echo "⚠️  Could not install 'rich' via pip. Workspace Manager visuals may be degraded."

# --- 3. ARMA DEVELOPMENT TOOLS ---
# HEMTT (The Builder)
if ! command -v hemtt &> /dev/null; then
    echo "🏗️  Installing HEMTT..."
    curl -s https://get.hemtt.dev | sh
    [ -f ./hemtt ] && sudo mv hemtt /usr/local/bin/
else
    echo "✅ HEMTT is already installed."
fi

# Mikero's Tools (Required for Mission Auditor)
echo "🔍 Setting up Mikero's Tools..."
if [ -f "./install_mikero.sh" ]; then
    bash ./install_mikero.sh
else
    echo "⚠️ install_mikero.sh not found. Skipping automatic install."
fi

# --- 4. GIT & SECURITY CONFIG ---
echo "🛡️  Verifying Security Posture..."
if ! gpg --list-secret-keys --keyid-format LONG | grep -q "sec"; then
    echo "ℹ️  Note: No GPG signing keys found. Remember to run 'gpg --full-generate-key' for Diamond commits."
else
    echo "✅ GPG Infrastructure ready."
fi

# Ensure git is configured for the unit standard
if [ -z "$(git config --global user.signingkey)" ]; then
    echo "ℹ️  Note: GPG signing is NOT enabled globally. Unit standard requires -S flag."
fi

echo " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo -e "\033[1;32m🎉 Setup Complete! Your environment is now Diamond Grade.\033[0m"
echo "   Run: ./tools/workspace_manager.py help"
echo " ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
