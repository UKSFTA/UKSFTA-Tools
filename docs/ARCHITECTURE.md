# UKSFTA Architecture & Standards

## 1. The Real Virtuality Engine VFS
The Arma 3 engine (Real Virtuality) utilizes a **Virtual File System (VFS)**. To prevent "Corrupt Mod" errors and performance bottlenecks, UKSFTA mods strictly adhere to these VFS standards:

### **Prefix Mapping ($PBOPREFIX$)**
Every PBO must contain a `$PBOPREFIX$` file in its root. This defines the virtual path (e.g., `z\uksfta\addons\main`). 
- **Requirement:** All internal references (includes, paths) must use the absolute virtual path. 
- **Validation:** Running `hemtt script check_prefixes` verifies this across the workspace.

## 2. Logic vs. Assets Split
To minimize update sizes and validation errors, we decouple code from binary assets.

### **The Brain: UKSFTA-Scripts**
- **Purpose:** Centralized logic core.
- **Registration:** Uses `CfgFunctions` for engine-native autoloading.
- **Macros:** Integrates with the **CBA Framework** via `script_macros.hpp`. This allows using `GVAR(variable)` which resolves to `UKSFTA_scripts_main_variable`.

### **The Body: UKSFTA-Mods & Maps**
- **Purpose:** 3D Models, Textures, and Terrains.
- **Interaction:** These projects should contain zero raw `.sqf` files. They "call" the Brain via functions.

## 3. Metadata Integrity
The Arma 3 Launcher is highly sensitive to `meta.cpp` inconsistencies.

- **Timestamp Standard:** We use the 64-bit Win32 FileTime epoch (100-nanosecond intervals since Jan 1, 1601).
- **PublishedID Enforcement:** The `publishedid` in `meta.cpp` must match the Steam Workshop ID exactly. Our `build.sh` enforces this automatically.
