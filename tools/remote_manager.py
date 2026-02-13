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
TOOLS_ROOT = Path(__file__).parent.parent.resolve()
REMOTE_ROOT = TOOLS_ROOT / "remote"
INVENTORY_PATH = REMOTE_ROOT / "inventory" / "nodes.json"
KEYS_DIR = REMOTE_ROOT / "keys"
SSH_KEY_NAME = "uksfta_vps"
DEVOPS_USER = "uksfta-admin"
REMOTE_WORKSPACE = "/opt/uksfta/workspace"

def ensure_dirs():
    KEYS_DIR.mkdir(exist_ok=True, parents=True)
    (REMOTE_ROOT / "inventory").mkdir(exist_ok=True, parents=True)

def get_inventory():
    if not INVENTORY_PATH.exists(): return {}
    try:
        with open(INVENTORY_PATH, "r") as f:
            data = json.load(f)
            return data.get("production_nodes", {}).get("hosts", {})
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

def run_ansible(playbook_name, node=None, extra_vars=None, dry_run=False):
    hosts = get_inventory()
    if not hosts: return False
    
    playbook = REMOTE_ROOT / "playbooks" / playbook_name
    ini_path = Path("/tmp/uksfta_temp_inventory.ini")
    managed_key_path = (KEYS_DIR / SSH_KEY_NAME).resolve()
    
    with open(ini_path, "w") as f:
        f.write("[production_nodes]\n")
        for name, data in hosts.items():
            if name == "example_vps": continue
            # Use absolute path for identity file to ensure rsync/ansible can always find it
            f.write(f"{name} ansible_host={data['ansible_host']} ansible_user={data['ansible_user']} ansible_ssh_private_key_file={managed_key_path}\n")

    env = os.environ.copy()
    env["ANSIBLE_CONFIG"] = str(REMOTE_ROOT / "ansible.cfg")
    
    ansible_cmd = ["ansible-playbook", "-i", str(ini_path), str(playbook)]
    if node: ansible_cmd.extend(["--limit", node])
    if dry_run: ansible_cmd.append("--check")
    if extra_vars:
        ev_str = " ".join([f"{k}={v}" for k, v in extra_vars.items()])
        ansible_cmd.extend(["--extra-vars", ev_str])
    
    try:
        res = subprocess.run(ansible_cmd, cwd=str(TOOLS_ROOT), env=env)
        return res.returncode == 0
    finally:
        if ini_path.exists(): os.remove(ini_path)

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
        table.add_column("Status", justify="center")
        
        for name, data in hosts.items():
            if name == "example_vps": continue
            active = "✅ OK" if is_node_provisioned(data['ansible_host']) else "❌ OFFLINE"
            table.add_row(name, data['ansible_host'], data['ansible_user'], active)
        console.print(table)
    else:
        print(f"{'NAME':<20} | {'HOST':<20} | {'USER':<15} | STATUS")
        print("-" * 75)
        for name, data in hosts.items():
            if name == "example_vps": continue
            status = "OK" if is_node_provisioned(data['ansible_host']) else "OFFLINE"
            print(f"{name:<20} | {data['ansible_host']:<20} | {data['ansible_user']:<15} | {status}")

def cmd_test(args):
    hosts = get_inventory()
    if not hosts: return
    print(f"📡 Testing connectivity for {len(hosts)} nodes...")
    for name, data in hosts.items():
        if name == "example_vps": continue
        host = data['ansible_host']
        res = is_node_provisioned(host)
        status = "✅ CONNECTED" if res else "❌ FAILED"
        print(f" • {name:<15} ({host}): {status}")

def cmd_provision(args):
    target = args.node if args.node else 'all'
    print(f"🚀 Provisioning software stack on: {target} {'[DRY-RUN]' if args.dry_run else ''}")
    run_ansible("provision.yml", node=args.node, dry_run=args.dry_run)

def cmd_sync_secrets(args):
    target = args.node if args.node else 'all'
    print(f"🔐 Synchronizing secrets to: {target} {'[DRY-RUN]' if args.dry_run else ''}")
    run_ansible("sync_secrets.yml", node=args.node, dry_run=args.dry_run)

def cmd_fetch_artifacts(args):
    target = args.node if args.node else 'all'
    print(f"📦 Fetching artifacts from: {target} {'[DRY-RUN]' if args.dry_run else ''}")
    run_ansible("fetch_artifacts.yml", node=args.node, dry_run=args.dry_run)

def cmd_monitor(args):
    target = args.node if args.node else 'all'
    print(f"📊 Querying health stats from: {target} {'[DRY-RUN]' if args.dry_run else ''}")
    run_ansible("monitor.yml", node=args.node, dry_run=args.dry_run)

def cmd_run(args):
    hosts = get_inventory()
    if not hosts: return
    
    target_name = args.node
    if not target_name or target_name not in hosts:
        target_name = next((n for n, d in hosts.items() if is_node_provisioned(d['ansible_host'])), None)
        if not target_name:
            print("❌ Error: No available production nodes found.")
            return
    
    target_data = hosts[target_name]
    host = target_data['ansible_host']
    managed_key_path = (KEYS_DIR / SSH_KEY_NAME).resolve()
    
    if not args.no_sync:
        print(f"🔄 Synchronizing workspace to {target_name} {'[DRY-RUN]' if args.dry_run else ''}...")
        # Pass the absolute TOOLS_ROOT to the playbook to ensure correct rsync source
        if not run_ansible("sync_workspace.yml", node=target_name, dry_run=args.dry_run, extra_vars={"local_root": str(TOOLS_ROOT) + "/"}):
            if not args.dry_run:
                print("❌ Synchronization failed. Aborting run.")
                return

    toolkit_cmd = " ".join(args.remote_args)
    print(f"🖥️  Executing on {target_name}: {toolkit_cmd} {'[DRY-RUN]' if args.dry_run else ''}")
    
    ssh_cmd = [
        "ssh", "-i", str(managed_key_path),
        f"{DEVOPS_USER}@{host}",
        f"cd {REMOTE_WORKSPACE} && python3 ./tools/workspace_manager.py {toolkit_cmd}"
    ]
    
    if args.dry_run:
        print(f"[DRY-RUN] Would run via SSH: {' '.join(ssh_cmd)}")
    else:
        subprocess.run(ssh_cmd)

def setup_node(connection_str, name=None, dry_run=False):
    ensure_dirs()
    key_path = generate_managed_key(); pub_key_path = key_path.with_suffix(".pub").resolve()
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
    success = run_ansible("setup_node.yml", node=None, extra_vars={"devops_user": DEVOPS_USER, "pub_key_path": str(pub_key_path)}, dry_run=dry_run)
    
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
    hosts = data.get("production_nodes", {}).get("hosts", {})
    existing_name = next((n for n, d in hosts.items() if d.get("ansible_host") == host), None)
    
    if dry_run:
        print("\n--- [DRY-RUN] Inventory Update ---")
        if existing_name and existing_name != name: print(f"ℹ️  Renaming existing node '{existing_name}' to '{name}'...")
        print(f"Node: {name} ({host})"); return

    if "example_vps" in hosts: del data["production_nodes"]["hosts"]["example_vps"]
    if existing_name and existing_name != name: del data["production_nodes"]["hosts"][existing_name]
    data["production_nodes"]["hosts"][name] = {"ansible_host": host, "ansible_user": DEVOPS_USER}
    with open(INVENTORY_PATH, "w") as f: json.dump(data, f, indent=4)
    print(f"✅ Added {name} ({host}) to inventory.")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Remote Node Manager")
    subparsers = parser.add_subparsers(dest="command")
    
    setup_p = subparsers.add_parser("setup", help="Onboard a new VPS node")
    setup_p.add_argument("host", help="Target host ([user@]host)")
    setup_p.add_argument("name", nargs="?", help="Logical name")
    setup_p.add_argument("--dry-run", action="store_true")
    
    subparsers.add_parser("list", help="List registered nodes")
    subparsers.add_parser("test", help="Test node connectivity")
    
    monitor_p = subparsers.add_parser("monitor", help="Check remote resource health")
    monitor_p.add_argument("--node")
    monitor_p.add_argument("--dry-run", action="store_true")
    
    secrets_p = subparsers.add_parser("sync-secrets", help="Deploy .env and unit signing keys")
    secrets_p.add_argument("--node")
    secrets_p.add_argument("--dry-run", action="store_true")
    
    fetch_p = subparsers.add_parser("fetch-artifacts", help="Pull remote builds to local")
    fetch_p.add_argument("--node")
    fetch_p.add_argument("--dry-run", action="store_true")
    
    prov_p = subparsers.add_parser("provision", help="Install production stack")
    prov_p.add_argument("--node")
    prov_p.add_argument("--dry-run", action="store_true")

    run_p = subparsers.add_parser("run", help="Delegate command to remote node")
    run_p.add_argument("--node")
    run_p.add_argument("--no-sync", action="store_true")
    run_p.add_argument("--dry-run", action="store_true")
    run_p.add_argument("remote_args", nargs=argparse.REMAINDER)

    args = parser.parse_args()
    if args.command == "setup": setup_node(args.host, args.name, dry_run=args.dry_run)
    elif args.command == "list": cmd_list(args)
    elif args.command == "test": cmd_test(args)
    elif args.command == "provision": cmd_provision(args)
    elif args.command == "monitor": cmd_monitor(args)
    elif args.command == "sync-secrets": cmd_sync_secrets(args)
    elif args.command == "fetch-artifacts": cmd_fetch_artifacts(args)
    elif args.command == "run": cmd_run(args)
    else: parser.print_help()

if __name__ == "__main__": main()
