#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import json
import subprocess
import argparse
from pathlib import Path

# Try to import rich for professional reporting
try:
    from rich.console import Console
    from rich.table import Table
    from rich import box
    USE_RICH = True
except ImportError:
    USE_RICH = False

# --- CONFIGURATION ---
REMOTE_ROOT = Path(__file__).parent.parent / "remote"
INVENTORY_PATH = REMOTE_ROOT / "inventory" / "nodes.json"
KEYS_DIR = REMOTE_ROOT / "keys"
SSH_KEY_NAME = "uksfta_vps"
DEVOPS_USER = "uksfta-admin"

def ensure_dirs():
    KEYS_DIR.mkdir(exist_ok=True, parents=True)
    (REMOTE_ROOT / "inventory").mkdir(exist_ok=True, parents=True)

def get_inventory():
    if not INVENTORY_PATH.exists(): return {}
    try:
        with open(INVENTORY_PATH, "r") as f:
            return json.load(f).get("production_nodes", {}).get("hosts", {})
    except: return {}

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

def is_node_provisioned(host):
    key_path = KEYS_DIR / SSH_KEY_NAME
    if not key_path.exists(): return False
    cmd = ["ssh", "-o", "ConnectTimeout=2", "-o", "BatchMode=yes", "-i", str(key_path), f"{DEVOPS_USER}@{host}", "echo 'OK'"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True)
        return "OK" in res.stdout
    except: return False

def cmd_list(args):
    hosts = get_inventory()
    if not hosts:
        print("❌ No nodes found in inventory.")
        return

    if USE_RICH:
        console = Console()
        table = Table(title="UKSFTA Distributed Production Nodes", box=box.ROUNDED, border_style="blue")
        table.add_column("Node Name", style="cyan")
        table.add_column("Host (IP)", style="magenta")
        table.add_column("User", style="green")
        table.add_column("Key Status", justify="center")
        
        for name, data in hosts.items():
            active = "✅ OK" if is_node_provisioned(data['ansible_host']) else "❌ OFFLINE"
            table.add_row(name, data['ansible_host'], data['ansible_user'], active)
        console.print(table)
    else:
        print(f"{'NAME':<20} | {'HOST':<20} | {'USER':<15} | STATUS")
        print("-" * 70)
        for name, data in hosts.items():
            status = "OK" if is_node_provisioned(data['ansible_host']) else "OFFLINE"
            print(f"{name:<20} | {data['ansible_host']:<20} | {data['ansible_user']:<15} | {status}")

def cmd_test(args):
    hosts = get_inventory()
    if not hosts: return
    print(f"📡 Testing connectivity for {len(hosts)} nodes...")
    for name, data in hosts.items():
        host = data['ansible_host']
        res = is_node_provisioned(host)
        status = "✅ CONNECTED" if res else "❌ FAILED (Key or Connection)"
        print(f" • {name:<15} ({host}): {status}")

def cmd_provision(args):
    hosts = get_inventory()
    if not hosts: return
    
    target = args.node if args.node else "all"
    if args.node and args.node not in hosts:
        print(f"❌ Error: Node '{args.node}' not found.")
        return

    print(f"🚀 Provisioning software stack on: {target}")
    playbook = REMOTE_ROOT / "playbooks" / "provision.yml"
    
    # We use the inventory file for provision
    # Note: Ansible needs a proper hosts format, we'll use our JSON inventory
    # For now, we'll convert nodes.json to a temporary INI for Ansible
    ini_path = Path("/tmp/uksfta_temp_inventory.ini")
    with open(ini_path, "w") as f:
        f.write("[production_nodes]\n")
        for name, data in hosts.items():
            f.write(f"{name} ansible_host={data['ansible_host']} ansible_user={data['ansible_user']} ansible_ssh_private_key_file={data['ansible_ssh_private_key_file']}\n")

    ansible_cmd = ["ansible-playbook", "-i", str(ini_path), str(playbook)]
    if args.node:
        ansible_cmd.extend(["--limit", args.node])
    
    subprocess.run(ansible_cmd)
    os.remove(ini_path)

def setup_node(connection_str, name=None, dry_run=False):
    ensure_dirs()
    key_path = generate_managed_key(); pub_key_path = key_path.with_suffix(".pub")
    if "@" in connection_str: initial_user, host = connection_str.split("@", 1)
    else: initial_user = "root"; host = connection_str
    if not name: name = host

    print(f"\n🔍 Auditing remote state for {host}...")
    if is_node_provisioned(host):
        print(f"✨ Node is already provisioned and accessible via managed key."); add_to_inventory(name, host, dry_run=dry_run)
        return

    print(f"\n--- Phase 1: Establish Initial Access ({host}) ---")
    if dry_run: print(f"[DRY-RUN] Would run: ssh-copy-id -o ConnectTimeout=10 {initial_user}@{host}")
    else:
        print(f"Connecting as {initial_user}... You will be prompted for the password.")
        try: subprocess.run(["ssh-copy-id", "-o", "ConnectTimeout=10", f"{initial_user}@{host}"], check=True)
        except subprocess.CalledProcessError: print("❌ Failed to copy SSH key."); return

    print("\n--- Phase 2: User Provisioning via Ansible ---")
    playbook = REMOTE_ROOT / "playbooks" / "setup_node.yml"
    ansible_cmd = ["ansible-playbook", "-i", f"{host},", "-u", initial_user, str(playbook), "--extra-vars", f"devops_user={DEVOPS_USER} pub_key_path={pub_key_path}"]
    if dry_run: print(f"[DRY-RUN] Would execute: {' '.join(ansible_cmd)}"); success = True
    else: res = subprocess.run(ansible_cmd); success = (res.returncode == 0)
    
    if success:
        print("\n--- Phase 3: Finalizing Inventory ---")
        add_to_inventory(name, host, dry_run=dry_run)
        msg = "✨ [DRY-RUN] Verification complete." if dry_run else f"✨ Node {name} is now production-ready!"
        print(f"\n{msg}")
    else: print("❌ Provisioning failed.")

def add_to_inventory(name, host, dry_run=False):
    data = {"production_nodes": {"hosts": {}}}
    if INVENTORY_PATH.exists():
        try:
            with open(INVENTORY_PATH, "r") as f: data = json.load(f)
        except: pass
    existing_name = next((n for n, d in data.get("production_nodes", {}).get("hosts", {}).items() if d.get("ansible_host") == host), None)
    node_data = {"ansible_host": host, "ansible_user": DEVOPS_USER, "ansible_ssh_private_key_file": f"remote/keys/{SSH_KEY_NAME}"}
    if dry_run:
        print("\n--- [DRY-RUN] Inventory Update ---")
        if existing_name and existing_name != name: print(f"ℹ️  Renaming existing node '{existing_name}' to '{name}'...")
        print(f"Node: {name} ({host})\n{json.dumps(node_data, indent=4)}"); return
    if existing_name and existing_name != name: del data["production_nodes"]["hosts"][existing_name]
    data["production_nodes"]["hosts"][name] = node_data
    with open(INVENTORY_PATH, "w") as f: json.dump(data, f, indent=4)
    print(f"✅ Added {name} ({host}) to inventory.")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Remote Node Manager")
    subparsers = parser.add_subparsers(dest="command")
    
    setup_p = subparsers.add_parser("setup", help="Onboard a new VPS node")
    setup_p.add_argument("host", help="Target host ([user@]host)")
    setup_p.add_argument("name", nargs="?", help="Logical name")
    setup_p.add_argument("--dry-run", action="store_true", help="Simulate setup")
    
    subparsers.add_parser("list", help="List registered nodes")
    subparsers.add_parser("test", help="Test node connectivity")
    
    prov_p = subparsers.add_parser("provision", help="Install production stack (SteamCMD, HEMTT)")
    prov_p.add_argument("--node", help="Target a specific node (default: all)")

    args = parser.parse_args()
    if args.command == "setup": setup_node(args.host, args.name, dry_run=args.dry_run)
    elif args.command == "list": cmd_list(args)
    elif args.command == "test": cmd_test(args)
    elif args.command == "provision": cmd_provision(args)
    else: parser.print_help()

if __name__ == "__main__": main()
