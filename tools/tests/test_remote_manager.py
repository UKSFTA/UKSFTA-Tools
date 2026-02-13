import unittest
from unittest.mock import patch, MagicMock, mock_open
import os
import json
import sys
from pathlib import Path

# Add parent dir to path so we can import remote_manager
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import remote_manager

class TestRemoteManager(unittest.TestCase):

    def setUp(self):
        remote_manager.INVENTORY_PATH = Path("/tmp/test_nodes.json")
        remote_manager.KEYS_DIR = Path("/tmp/test_keys")
        if remote_manager.INVENTORY_PATH.exists():
            os.remove(remote_manager.INVENTORY_PATH)

    def test_add_to_inventory(self):
        # Scenario: Adding a new node
        remote_manager.add_to_inventory("test-node", "1.2.3.4", dry_run=False)
        
        self.assertTrue(remote_manager.INVENTORY_PATH.exists())
        with open(remote_manager.INVENTORY_PATH, "r") as f:
            data = json.load(f)
            self.assertIn("test-node", data["production_nodes"]["hosts"])
            self.assertEqual(data["production_nodes"]["hosts"]["test-node"]["ansible_host"], "1.2.3.4")

    def test_add_to_inventory_rename(self):
        # Scenario: Renaming an existing host IP
        initial_data = {
            "production_nodes": {
                "hosts": {
                    "old-name": {
                        "ansible_host": "1.2.3.4",
                        "ansible_user": "root"
                    }
                }
            }
        }
        with open(remote_manager.INVENTORY_PATH, "w") as f:
            json.dump(initial_data, f)

        remote_manager.add_to_inventory("new-name", "1.2.3.4", dry_run=False)

        with open(remote_manager.INVENTORY_PATH, "r") as f:
            data = json.load(f)
            self.assertIn("new-name", data["production_nodes"]["hosts"])
            self.assertNotIn("old-name", data["production_nodes"]["hosts"])

    @patch("subprocess.run")
    def test_generate_managed_key(self, mock_run):
        # Scenario: Key doesn't exist, should generate
        with patch("pathlib.Path.exists", return_value=False):
            remote_manager.generate_managed_key()
            mock_run.assert_called()
            self.assertIn("ssh-keygen", mock_run.call_args[0][0])

    @patch("subprocess.run")
    def test_setup_node_dry_run(self, mock_run):
        # Scenario: Dry run setup should not call ssh-copy-id or ansible
        remote_manager.setup_node("root@1.2.3.4", name="test", dry_run=True)
        
        # Verify no network/system calls were made
        for call in mock_run.call_args_list:
            cmd = call[0][0]
            self.assertNotIn("ssh-copy-id", cmd)
            self.assertNotIn("ansible-playbook", cmd)

if __name__ == "__main__":
    unittest.main()
