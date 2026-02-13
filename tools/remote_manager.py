#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import json
import subprocess
import argparse
import getpass
from pathlib import Path

# --- CONFIGURATION ---
REMOTE_ROOT = Path(__file__).parent.parent / "remote"
INVENTORY_PATH = REMOTE_ROOT / "inventory" / "nodes.json"
KEYS_DIR = REMOTE_ROOT / "keys"
SSH_KEY_NAME = "uksfta_vps"
DEVOPS_USER = "uksfta-admin"

def ensure_dirs():
    KEYS_DIR.mkdir(exist_ok=True, parents=True)
    (REMOTE_ROOT / "inventory").mkdir(exist_ok=True, parents=True)

def generate_managed_key():
    key_path = KEYS_DIR / SSH_KEY_NAME
    if not key_path.exists():
        print(f"🔑 Generating managed unit key: {SSH_KEY_NAME}")
        subprocess.run([
            "ssh-keygen", "-t", "ed25519", 
            "-f", str(key_path), 
            "-N", "", 
            "-C", "uksfta-devops-managed"
        ], check=True)
    return key_path

def add_to_inventory(name, host):
    data = {"production_nodes": {"hosts": {}}}
    if INVENTORY_PATH.exists():
        with open(INVENTORY_PATH, "r") as f:
            data = json.load(f)
    
    data["production_nodes"]["hosts"][name] = {
        "ansible_host": host,
        "ansible_user": DEVOPS_USER,
        "ansible_ssh_private_key_file": f"remote/keys/{SSH_KEY_NAME}"
    }
    
    with open(INVENTORY_PATH, "w") as f:
        json.dump(data, f, indent=4)
    print(f"✅ Added {name} ({host}) to inventory.")

def setup_node(name, host, initial_user):
    ensure_dirs()
    key_path = generate_managed_key()
    pub_key_path = key_path.with_suffix(".pub")

    print(f"
--- Phase 1: Establish Initial Access ({host}) ---")
    print(f"Connecting as {initial_user}... You will be prompted for the password.")
    
    # Copy the local user's pubkey so we can run Ansible
    # We use subprocess.run to allow the user to interact with the password prompt
    try:
        subprocess.run(["ssh-copy-id", "-o", "ConnectTimeout=10", f"{initial_user}@{host}"], check=True)
    except subprocess.CalledProcessError:
        print("❌ Failed to copy SSH key. Ensure initial user has password auth enabled.")
        return

    print("
--- Phase 2: User Provisioning via Ansible ---")
    # Run the setup playbook
    # We use the current user's default SSH key for this initial run
    playbook = REMOTE_ROOT / "playbooks" / "setup_node.yml"
    ansible_cmd = [
        "ansible-playbook", 
        "-i", f"{host},", 
        "-u", initial_user, 
        str(playbook),
        "--extra-vars", f"devops_user={DEVOPS_USER} pub_key_path={pub_key_path}"
    ]
    
    res = subprocess.run(ansible_cmd)
    
    if res.returncode == 0:
        print("
--- Phase 3: Finalizing Inventory ---")
        add_to_inventory(name, host)
        print(f"
✨ Node {name} is now production-ready!")
        print(f"Dedicated User: {DEVOPS_USER}")
        print(f"Access Method: Managed SSH Key ({SSH_KEY_NAME})")
    else:
        print("❌ Provisioning failed during Ansible execution.")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Remote Node Manager")
    subparsers = parser.add_subparsers(dest="command")

    # Setup command
    setup_p = subparsers.add_parser("setup", help="Onboard a new VPS node")
    setup_p.add_argument("name", help="Logical name for the server (e.g. uk-prod-1)")
    setup_p.add_argument("host", help="IP address or domain of the VPS")
    setup_p.add_argument("--user", default="root", help="Initial user for setup (default: root)")

    args = parser.parse_args()

    if args.command == "setup":
        setup_node(args.name, args.host, args.user)
    else:
        parser.print_help()

if __name__ == "__main__":
    main()
