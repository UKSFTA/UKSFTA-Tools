#!/usr/bin/env python3
# -*- coding: utf-8 -*-
import os
import sys
import json
import subprocess
import argparse
from pathlib import Path
from datetime import datetime

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
        subprocess.run(["ssh-keygen", "-t", "ed25519", "-f", str(key_path), "-N", "", "-C", "uksfta-devops-managed"], check=True)
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
            f.write(f"{name} ansible_host={data['ansible_host']} ansible_user={data['ansible_user']} ansible_ssh_private_key_file={managed_key_path}\n")
    env = os.environ.copy(); env["ANSIBLE_CONFIG"] = str(REMOTE_ROOT / "ansible.cfg")
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
    if not hosts: print("❌ No nodes found."); return
    if USE_RICH:
        console = Console(); table = Table(title="UKSFTA Distributed Production Nodes", box=box.ROUNDED, border_style="blue")
        table.add_column("Node Name", style="cyan"); table.add_column("Host (IP)", style="magenta"); table.add_column("User", style="green"); table.add_column("Status", justify="center")
        for name, data in hosts.items():
            if name == "example_vps": continue
            active = "✅ OK" if is_node_provisioned(data['ansible_host']) else "❌ OFFLINE"
            table.add_row(name, data['ansible_host'], data['ansible_user'], active)
        console.print(table)
    else:
        for name, data in hosts.items():
            if name == "example_vps": continue
            print(f"{name:<20} | {data['ansible_host']:<20} | OK" if is_node_provisioned(data['ansible_host']) else f"{name:<20} | {data['ansible_host']:<20} | OFFLINE")

def cmd_provision(args):
    print(f"🚀 Provisioning stack on: {args.node if args.node else 'all'} {'[DRY-RUN]' if args.dry_run else ''}")
    run_ansible("provision.yml", node=args.node, dry_run=args.dry_run)

def cmd_sync_secrets(args):
    print(f"🔐 Synchronizing secrets to: {args.node if args.node else 'all'} {'[DRY-RUN]' if args.dry_run else ''}")
    run_ansible("sync_secrets.yml", node=args.node, dry_run=args.dry_run)

def cmd_fetch_artifacts(args):
    print(f"📦 Fetching artifacts from: {args.node if args.node else 'all'} {'[DRY-RUN]' if args.dry_run else ''}")
    run_ansible("fetch_artifacts.yml", node=args.node, dry_run=args.dry_run)

def cmd_monitor(args):
    print(f"📊 Monitoring nodes: {args.node if args.node else 'all'} {'[DRY-RUN]' if args.dry_run else ''}")
    run_ansible("monitor.yml", node=args.node, dry_run=args.dry_run)

def cmd_backup(args):
    target = args.node if args.node else 'all'
    if args.action == 'setup':
        print(f"📦 Initializing Borg Repository on: {target}")
        run_ansible("backup_ops.yml", node=args.node, extra_vars={"operation": "setup"})
    elif args.action == 'create':
        archive_name = args.name if args.name else f"workspace_{datetime.now().strftime('%Y%m%d_%H%M%S')}"
        print(f"📦 Creating Deduplicated Archive: {archive_name} on {target}")
        run_ansible("backup_ops.yml", node=args.node, extra_vars={"operation": "create", "archive_name": archive_name})
    elif args.action == 'list':
        print(f"📂 Listing Archives on: {target}")
        run_ansible("backup_ops.yml", node=args.node, extra_vars={"operation": "list"})

def cmd_run(args):
    hosts = get_inventory(); target_name = args.node
    if not target_name or target_name not in hosts:
        target_name = next((n for n, d in hosts.items() if is_node_provisioned(d['ansible_host'])), None)
        if not target_name: print("❌ No nodes found."); return
    target_data = hosts[target_name]; host = target_data['ansible_host']; managed_key_path = (KEYS_DIR / SSH_KEY_NAME).resolve()
    if not args.no_sync:
        print(f"🔄 Syncing workspace to {target_name} {'[DRY-RUN]' if args.dry_run else ''}...")
        if not run_ansible("sync_workspace.yml", node=target_name, dry_run=args.dry_run, extra_vars={"local_root": str(TOOLS_ROOT) + "/"}):
            if not args.dry_run: print("❌ Sync failed."); return
    toolkit_cmd = " ".join(args.remote_args)
    print(f"🖥️  Executing on {target_name}: {toolkit_cmd} {'[DRY-RUN]' if args.dry_run else ''}")
    ssh_cmd = ["ssh", "-i", str(managed_key_path), f"{DEVOPS_USER}@{host}", f"cd {REMOTE_WORKSPACE} && python3 ./tools/workspace_manager.py {toolkit_cmd}"]
    if args.dry_run: print(f"[DRY-RUN] SSH: {' '.join(ssh_cmd)}")
    else: subprocess.run(ssh_cmd)

def setup_node(connection_str, name=None, dry_run=False):
    ensure_dirs(); key_path = generate_managed_key(); pub_key_path = key_path.with_suffix(".pub").resolve()
    if "@" in connection_str: initial_user, host = connection_str.split("@", 1)
    else: initial_user = "root"; host = connection_str
    if not name: name = host
    print(f"\n🔍 Auditing state for {host}...")
    if is_node_provisioned(host): print(f"✨ Already provisioned."); add_to_inventory(name, host, dry_run=dry_run); return
    print(f"\n--- Phase 1: Access ({host}) ---")
    if dry_run: print(f"[DRY-RUN] Would run: ssh-copy-id {initial_user}@{host}")
    else:
        try: subprocess.run(["ssh-copy-id", "-o", "ConnectTimeout=10", f"{initial_user}@{host}"], check=True)
        except: print("❌ Failed key copy."); return
    print("\n--- Phase 2: Provisioning ---")
    success = run_ansible("setup_node.yml", node=None, extra_vars={"devops_user": DEVOPS_USER, "pub_key_path": str(pub_key_path)}, dry_run=dry_run) if not dry_run else True
    if success: add_to_inventory(name, host, dry_run=dry_run); print(f"\n✨ Node {name} ready!")
    else: print("❌ Failed.")

def add_to_inventory(name, host, dry_run=False):
    data = {"production_nodes": {"hosts": {}}}
    if INVENTORY_PATH.exists():
        try:
            with open(INVENTORY_PATH, "r") as f: data = json.load(f)
        except: pass
    hosts = data.get("production_nodes", {}).get("hosts", {})
    existing_name = next((n for n, d in hosts.items() if d.get("ansible_host") == host), None)
    if dry_run: print(f"Node: {name} ({host})"); return
    if "example_vps" in hosts: del data["production_nodes"]["hosts"]["example_vps"]
    if existing_name and existing_name != name: del data["production_nodes"]["hosts"][existing_name]
    data["production_nodes"]["hosts"][name] = {"ansible_host": host, "ansible_user": DEVOPS_USER}
    with open(INVENTORY_PATH, "w") as f: json.dump(data, f, indent=4)
    print(f"✅ Added {name} to inventory.")

def main():
    parser = argparse.ArgumentParser(description="UKSFTA Remote Manager")
    subparsers = parser.add_subparsers(dest="command")
    setup_p = subparsers.add_parser("setup", help="Onboard VPS"); setup_p.add_argument("host"); setup_p.add_argument("name", nargs="?"); setup_p.add_argument("--dry-run", action="store_true")
    subparsers.add_parser("list", help="List nodes"); subparsers.add_parser("test", help="Test connectivity")
    subparsers.add_parser("monitor", help="Check health").add_argument("--node"); subparsers.add_parser("sync-secrets", help="Sync keys").add_argument("--node")
    subparsers.add_parser("fetch-artifacts", help="Fetch builds").add_argument("--node")
    prov_p = subparsers.add_parser("provision", help="Install stack"); prov_p.add_argument("--node"); prov_p.add_argument("--dry-run", action="store_true")
    backup_p = subparsers.add_parser("backup", help="Archive workspace"); backup_p.add_argument("action", choices=['setup', 'create', 'list']); backup_p.add_argument("--node"); backup_p.add_argument("--name", help="Archive name")
    run_p = subparsers.add_parser("run", help="Delegate command"); run_p.add_argument("--node"); run_p.add_argument("--no-sync", action="store_true"); run_p.add_argument("--dry-run", action="store_true"); run_p.add_argument("remote_args", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command == "setup": setup_node(args.host, args.name, dry_run=args.dry_run)
    elif args.command == "list": cmd_list(args)
    elif args.command == "test": cmd_test(args)
    elif args.command == "provision": cmd_provision(args)
    elif args.command == "monitor": cmd_monitor(args)
    elif args.command == "sync-secrets": cmd_sync_secrets(args)
    elif args.command == "fetch-artifacts": cmd_fetch_artifacts(args)
    elif args.command == "backup": cmd_backup(args)
    elif args.command == "run": cmd_run(args)
    else: parser.print_help()

if __name__ == "__main__": main()
