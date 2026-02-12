# Environment Setup Guide

Follow these steps to initialize a professional UKSFTA development environment on Linux.

## 1. System Dependencies

Ensure the following base packages are installed:

```bash
sudo apt update && sudo apt install -y python3 python3-pip steamcmd zip git
```

## 2. Python Environment

Install the `rich` library for the visual dashboard:

```bash
pip install rich pytest
```

## 3. Mikero Tools (DePbo Suite)

We use version **0.10.13+**.

1. **Shared Library:** `libdepbo.so.0` must be in `$HOME/lib/`.
2. **Binaries:** `makepbo`, `extractpbo`, and `rapify` must be in `$HOME/bin/`.
3. **Library Path:** Ensure your `~/.bashrc` includes:

   ```bash
   export LD_LIBRARY_PATH=$LD_LIBRARY_PATH:$HOME/lib
   export PATH=$PATH:$HOME/bin
   ```

## 4. UKSFTA Workspace Initialization

1. Create a parent development directory: `mkdir UKSFTA-Dev && cd UKSFTA-Dev`
2. Clone the tools: `git clone git@github.com:UKSFTA/UKSFTA-Tools.git`
3. Run the bootstrap: `cd UKSFTA-Tools && ./bootstrap.sh`
4. Use the `depbo` command to verify all tools are accessible.
