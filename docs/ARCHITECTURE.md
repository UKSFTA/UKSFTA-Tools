# UKSFTA Architecture & Standards

## 1. The Real Virtuality Engine VFS

The Arma 3 engine (Real Virtuality) utilizes a **Virtual File System (VFS)**. To prevent "Corrupt Mod" errors and performance bottlenecks, UKSFTA mods strictly adhere to these VFS standards:

### **Prefix Mapping ($PBOPREFIX$)**

Every PBO must contain a `$PBOPREFIX$` file in its root. This defines the virtual path (e.g., `z\uksfta\main`).

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

## 4. Build System & Tooling

UKSFTA uses a custom `build.sh` wrapper around HEMTT for binarization, signing, and staging.

### Build Commands

| Command | Type | Purpose |
| :--- | :--- | :--- |
| `./build.sh dev` | **VFS** | Fastest development loop; uses symlinks and Virtual File System. |
| `./build.sh fast` | **Solid** | Full PBO packing but skips slow binarization (`--no-bin`). |
| `./build.sh build` | **Solid** | Standard binarized build for local testing. |
| `./build.sh release` | **Gold** | Production-ready build with full optimization and signing. |

### Performance Optimization

The build system automatically optimizes thread usage to prevent system lockups:

- **Default:** Uses `(nproc - 2)` threads to maintain UI/OS responsiveness.
- **Manual Override:** Use the `--threads` or `-t` flag to specify a custom thread count:

```bash
./build.sh build --threads 16
```

### Temporary File Management

To avoid disk quota issues on shared filesystems (like `/tmp`), the system redirects all temporary file operations to the project-local `.hemttout/tmp` directory via:

- `HEMTT_TEMP_DIR`
- `TMPDIR`, `TEMP`, `TMP`

