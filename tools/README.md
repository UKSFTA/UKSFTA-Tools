# UKSFTA Mod Manager

A simple, portable Python tool for managing Arma 3 Steam Workshop mods in HEMTT-based projects.

## Features

- **Easy Configuration**: Just paste Workshop URLs or IDs into `mod_sources.txt`.
- **Automatic Sync**: Downloads, updates, and copies mods to your project.
- **Smart Cleanup**: Automatically removes files when a mod is removed from the list.
- **Key Management**: Automatically extracts `.bikey` files for server signing.
- **Anonymous**: No Steam account credentials required for public mods.

## Setup

1.  Ensure you have `steamcmd` installed on your system.
    *   **Linux**: `sudo apt install steamcmd` (or equivalent).
    *   **Windows**: Download from Valve and add to your PATH.
2.  Copy `tools/manage_mods.py` to your project's `tools/` directory.
3.  Create a `mod_sources.txt` in the root of your project.

## Usage

1.  **Add Mods**:
    Open `mod_sources.txt` and add Steam Workshop links or IDs, one per line.
    ```text
    https://steamcommunity.com/sharedfiles/filedetails/?id=450814997
    463939057
    ```

2.  **Run the Tool**:
    ```bash
    python3 tools/manage_mods.py
    ```

    This will:
    *   Download the mods using `steamcmd`.
    *   Copy `.pbo` and `.bisign` files to `addons/`.
    *   Copy `.bikey` files to `keys/`.
    *   Update `mods.lock` to track installed files.

3.  **Remove Mods**:
    Simply delete the line from `mod_sources.txt` and run the script again. The tool will delete the associated files from `addons/` and `keys/`.

## Requirements

*   Python 3.6+
*   SteamCMD

## Troubleshooting

*   **"SteamCMD not found"**: Ensure `steamcmd` is in your system PATH.
*   **Permissions**: On Linux, ensure the script has write access to `addons/` and `keys/`.
