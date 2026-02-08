#!/bin/bash

# UKSFTA Development Environment Bootstrap
# This script installs the necessary tools for UKSFTA mod development.

echo "UKSF Taskforce Alpha - DevOps Bootstrapper"
echo "=========================================="

# 1. Check for Python
if ! command -v python3 &> /dev/null; then
    echo "Error: python3 is not installed. Please install it first."
    exit 1
fi

# 2. Install HEMTT (Linux)
if ! command -v hemtt &> /dev/null; then
    echo "Installing HEMTT..."
    curl -s https://api.github.com/repos/vurtual/hemtt/releases/latest | 
    grep "browser_download_url.*hemtt_linux_x86_64.zip" | 
    cut -d : -f 2,3 | 
    tr -d " | 
    wget -qi - -O hemtt.zip
    unzip hemtt.zip
    chmod +x hemtt
    sudo mv hemtt /usr/local/bin/
    rm hemtt.zip
    echo "HEMTT installed."
else
    echo "HEMTT already installed: $(hemtt --version)"
fi

# 3. Install SteamCMD (Linux)
if ! command -v steamcmd &> /dev/null; then
    echo "Installing SteamCMD..."
    sudo apt-get update
    sudo apt-get install -y steamcmd
    # Create symlink for easier access
    sudo ln -s /usr/games/steamcmd /usr/local/bin/steamcmd
    echo "SteamCMD installed."
else
    echo "SteamCMD already installed."
fi

# 4. Install GitHub CLI (Linux)
if ! command -v gh &> /dev/null; then
    echo "Installing GitHub CLI..."
    type -p curl >/dev/null || (sudo apt update && sudo apt install curl -y)
    sudo mkdir -p -m 755 /etc/apt/keyrings
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | sudo dd of=/etc/apt/keyrings/githubcli-archive-keyring.gpg
    sudo chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null
    sudo apt update
    sudo apt install gh -y
    echo "GitHub CLI installed."
else
    echo "GitHub CLI already installed."
fi

echo ""
echo "Setup complete! You are ready to develop for UKSFTA."
echo "Next steps: Run './sync_tools.sh' in your development root."
