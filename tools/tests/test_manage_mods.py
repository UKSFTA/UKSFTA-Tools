import unittest
from unittest.mock import patch, mock_open, MagicMock
import os
import json
import shutil
import sys

# Add parent dir to path so we can import manage_mods
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import manage_mods

class TestManageMods(unittest.TestCase):

    def test_get_mod_ids_from_file(self):
        content = """
        https://steamcommunity.com/sharedfiles/filedetails/?id=12345678
        87654321
        # Some comment
        id=11223344
        @ignore 99999999
        """
        with patch("builtins.open", mock_open(read_data=content)):
            with patch("os.path.exists", return_value=True):
                ids = manage_mods.get_mod_ids_from_file()
                self.assertIn("12345678", ids)
                self.assertIn("87654321", ids)
                self.assertIn("11223344", ids)
                self.assertNotIn("99999999", ids)

    def test_get_ignored_ids_from_file(self):
        content = """
        # Sources
        12345678
        @ignore 99999999 88888888
        ignore=77777777
        """
        with patch("builtins.open", mock_open(read_data=content)):
            with patch("os.path.exists", return_value=True):
                ignored = manage_mods.get_ignored_ids_from_file()
                self.assertIn("99999999", ignored)
                self.assertIn("88888888", ignored)
                self.assertIn("77777777", ignored)
                self.assertNotIn("12345678", ignored)

    @patch("manage_mods.get_workshop_metadata")
    def test_resolve_dependencies_with_ignore(self, mock_meta):
        # Setup mock metadata
        # 123 depends on 456
        # 456 depends on 789
        mock_meta.side_effect = [
            {"name": "Mod 123", "dependencies": [{"id": "456", "name": "Mod 456"}]},
            {"name": "Mod 456", "dependencies": [{"id": "789", "name": "Mod 789"}]},
            {"name": "Mod 789", "dependencies": []}
        ]
        
        initial = {"123": "Tag"}
        ignored = {"789"}
        
        resolved = manage_mods.resolve_dependencies(initial, ignored)
        
        self.assertIn("123", resolved)
        self.assertIn("456", resolved)
        self.assertNotIn("789", resolved)

    @patch("os.remove")
    @patch("json.load")
    @patch("os.path.exists")
    @patch("builtins.open", new_callable=mock_open)
    def test_sync_mods_cleanup(self, mock_file, mock_exists, mock_load, mock_remove):
        # Scenario: Mod 999 was installed (has lock entry), but is now ignored or removed.
        manage_mods.LOCK_FILE = "mods.lock"
        
        # Mocking existence checks
        def side_effect_exists(path):
            if path == "mods.lock": return True
            if "addons/" in path: return True
            return False
        mock_exists.side_effect = side_effect_exists
        
        # Lock file has entry for 999
        mock_load.return_value = {
            "mods": {
                "999": {
                    "files": ["addons/old_mod.pbo"],
                    "name": "Old Mod",
                    "dependencies": []
                }
            }
        }
        
        # Run sync with EMPTY resolved_info
        # We need to bypass the workshop directory search for this test
        with patch("os.path.expanduser", return_value="/tmp"):
            with patch("os.makedirs"):
                manage_mods.sync_mods({})
        
        # Verify removal was called for the file belonging to mod 999
        mock_remove.assert_called_with("addons/old_mod.pbo")

if __name__ == "__main__":
    unittest.main()
