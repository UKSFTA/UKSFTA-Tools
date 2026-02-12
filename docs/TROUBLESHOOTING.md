# Troubleshooting Guide

## 1. Arma 3 Launcher: "Corrupt Mod"

This is almost never actual file corruption. It is usually a **Metadata Mismatch**.

- **Fix:** Ensure the `publishedid` in `meta.cpp` matches the Workshop ID exactly.
- **Solution:** Re-run `./tools/workspace_manager.py release` to re-inject the IDs.

## 2. Steam: Redownload Loops

Common on **Btrfs** filesystems when large PBOs are being written at high speed.

- **Diagnosis:** Check `content_log.txt` for "Staged file validation failed."
- **Fix:**
  1. Exit Steam.
  2. `rm /ext/SteamLibrary/steamapps/appmanifest_107410.acf`
  3. `rm -rf /ext/SteamLibrary/steamapps/downloading/107410`
  4. Restart Steam and let it "Discover Existing Files."

## 3. HEMTT: "File not found" for CBA Macros

HEMTT cannot see your P Drive.

- **Fix:** Ensure your project has an `include/x/cba` structure.
- **Solution:** Run `./tools/workspace_manager.py update` to sync the latest CBA include templates.

## 4. Linux Server: "File Case Mismatch"

Arma 3 servers are case-sensitive.

- **Fix:** All PBOs should be lowercase.
- **Solution:** Our `mod_integrity_checker.py` will flag any illegal characters or non-ASCII filenames.
