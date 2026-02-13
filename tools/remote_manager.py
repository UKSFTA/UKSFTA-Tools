#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import json
import subprocess
import argparse
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

def add_to_inventory(name, host, dry_run=False):
    data = {"production_nodes": {"hosts": {}}}
    if INVENTORY_PATH.exists():
        try:
            with open(INVENTORY_PATH, "r") as f: data = json.load(f)
        except: pass
    
    node_data = {
        "ansible_host": host,
        "ansible_user": DEVOPS_USER,
        "ansible_ssh_private_key_file": f"remote/keys/{SSH_KEY_NAME}"
    }
    
    if dry_run:
        print("\n--- [DRY-RUN] Inventory Update ---")
        print(f"Node: {name} ({host})")
        print(json.dumps(node_data, indent=4))
        return

    data["production_nodes"]["hosts"][name] = node_data
    with open(INVENTORY_PATH, "w") as f: json.dump(data, f, indent=4)
    print(f"✅ Added {name} ({host}) to inventory.")

def setup_node(connection_str, name=None, dry_run=False):
    ensure_dirs()
    key_path = generate_managed_key()
    pub_key_path = key_path.with_suffix(".pub")

    # Parse user@host
    if "@" in connection_str:
        initial_user, host = connection_str.split("@", 1)
    else:
        initial_user = "root"
        host = connection_str
    
    # Auto-generate name if not provided
    if not name:
        name = host

    print(f"\n--- Phase 1: Establish Initial Access ({host}) ---")
    if dry_run:
        print(f"[DRY-RUN] Would run: ssh-copy-id -o ConnectTimeout=10 {initial_user}@{host}")
    else:
        print(f"Connecting as {initial_user}... You will be prompted for the password.")
        try:
            subprocess.run(["ssh-copy-id", "-o", "ConnectTimeout=10", f"{initial_user}@{host}"], check=True)
        except subprocess.CalledProcessError:
            print("❌ Failed to copy SSH key.")
            return

    print("\n--- Phase 2: User Provisioning via Ansible ---")
    playbook = REMOTE_ROOT / "playbooks" / "setup_node.yml"
    ansible_cmd = [
        "ansible-playbook", "-i", f"{host},", "-u", initial_user, str(playbook),
        "--extra-vars", f"devops_user={DEVOPS_USER} pub_key_path={pub_key_path}"
    ]
    
    if dry_run:
        print(f"[DRY-RUN] Would execute: {' '.join(ansible_cmd)}")
        success = True
    else:
        res = subprocess.run(ansible_cmd)
        success = (res.returncode == 0)
    
    if success:
        print("\n--- Phase 3: Finalizing Inventory ---")
        add_to_inventory(name, host, dry_run=dry_run)
        msg = "✨ [DRY-RUN] Verification complete." if dry_run else f"✨ Node {name} is now production-ready!"
        print(f"\n{msg}")
    else:
        print("❌ Provisioning failed.")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Remote Node Manager")
    subparsers = parser.add_subparsers(dest="command")
    
    setup_p = subparsers.add_parser("setup", help="Onboard a new VPS node")
    setup_p.add_argument("host", help="Target host (format: [user@]host)")
    setup_p.add_argument("name", nargs="?", help="Logical name for server (optional, defaults to host)")
    setup_p.add_argument("--dry-run", action="store_true", help="Simulate setup")
    
    args = parser.parse_args()
    if args.command == "setup":
        setup_node(args.host, args.name, dry_run=args.dry_run)
    else:
        parser.print_help()

if __name__ == "__main__": main()
