# Environment Setup Guide

Follow these steps to initialize a professional UKSFTA development environment on Linux.

## 1. System Dependencies

Ensure the following base packages are installed:

```bash
# Ubuntu/Debian
sudo apt update && sudo apt install -y python3 python3-pip steamcmd zip git ansible ansible-lint rsync

# Arch Linux
sudo pacman -S python-pip steamcmd zip git ansible ansible-lint rsync
```

## 2. Python Environment

Install the necessary libraries for the visual dashboard and Steam API:

```bash
# We recommend using system packages on Debian/Ubuntu to avoid PEP 668 issues:
sudo apt install python3-rich python3-requests python3-pytest

# Or via pip if your environment allows:
pip install rich requests pytest
```

## 3. DevOps Suite Initialization

1. Create a parent development directory: `mkdir UKSFTA-Dev && cd UKSFTA-Dev`
2. Clone the tools: `git clone git@github.com:UKSFTA/UKSFTA-Tools.git`
3. Run the bootstrap: `cd UKSFTA-Tools && ./bootstrap.sh`
4. Verify environment: `./tools/workspace_manager.py check-env`

## 4. Distributed Infrastructure (Optional)

If you intend to use remote VPS nodes for builds or synchronization:

1. **Onboard Server**: `./tools/workspace_manager.py remote setup user@host server-name`
2. **Deploy Stack**: `./tools/workspace_manager.py remote provision --node server-name`
3. **Sync Secrets**: Ensure your local `.env` is populated, then run `./tools/workspace_manager.py remote sync-secrets`

---

### Production Standard

- **GPG Signing**: All commits MUST be signed.
- **Python**: Ensure you are using Python 3.11+.
- **SSH**: Managed keys are stored in `remote/keys/`. DO NOT share or commit these.
