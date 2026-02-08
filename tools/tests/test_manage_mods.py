import unittest
import os
import sys
import json
from unittest.mock import patch, MagicMock, mock_open

# Add parent directory to path to import the script
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import manage_mods

class TestManageMods(unittest.TestCase):

    def test_get_mod_ids(self):
        content = """
        https://steamcommunity.com/sharedfiles/filedetails/?id=12345678
        87654321
        # Some comment
        id=11223344
        """
        with patch("builtins.open", mock_open(read_data=content)):
            with patch("os.path.exists", return_value=True):
                ids = manage_mods.get_mod_ids()
                self.assertEqual(ids, {"12345678", "87654321", "11223344"})

    def test_get_mod_ids_no_file(self):
        with patch("os.path.exists", return_value=False):
            ids = manage_mods.get_mod_ids()
            self.assertEqual(ids, set())

    @patch("subprocess.run")
    def test_run_steamcmd(self, mock_run):
        ids = {"123", "456"}
        manage_mods.run_steamcmd(ids)
        
        args, _ = mock_run.call_args
        cmd = args[0]
        self.assertEqual(cmd[0], "steamcmd")
        self.assertIn("+login", cmd)
        self.assertIn("anonymous", cmd)
        self.assertIn("+workshop_download_item", cmd)
        self.assertIn("123", cmd)
        self.assertIn("456", cmd)

    @patch("os.makedirs")
    @patch("shutil.copy2")
    @patch("os.walk")
    @patch("os.path.exists")
    @patch("builtins.open", new_callable=mock_open)
    @patch("json.dump")
    def test_sync_mods_install(self, mock_json_dump, mock_file, mock_exists, mock_walk, mock_copy, mock_makedirs):
        # Setup mocks
        mock_exists.return_value = True # Assume all paths exist
        
        # Mock os.walk to simulate a mod directory
        # Structure: root, dirs, files
        mock_walk.return_value = [
            ("/path/to/mod/123", [], ["mod.pbo", "mod.bisign", "mod.bikey", "readme.txt"])
        ]
        
        # Mock lock file loading (empty initially)
        # We need to handle open() being called multiple times (read lock, write lock)
        # Since we are mocking open globally, we verify behavior via side_effects or context
        # But for simple logic verification, assuming no existing lock file is easier first.
        # existing lock file check:
        # if os.path.exists(LOCK_FILE): -> We can mock this specific call if needed, 
        # but mock_exists returns True for everything. 
        # So it will try to read the lock file. We need the read to return valid JSON.
        
        # Let's verify the copy logic primarily.
        
        manage_mods.LOCK_FILE = "dummy.lock"
        
        # We need to be careful with mock_open and json.load reading from it
        # Simplest way is to mock json.load directly
        with patch("json.load", return_value={}) as mock_json_load:
            manage_mods.sync_mods({"123"})
            
            # Check if directories created
            mock_makedirs.assert_any_call(manage_mods.ADDONS_DIR, exist_ok=True)
            mock_makedirs.assert_any_call(manage_mods.KEYS_DIR, exist_ok=True)
            
            # Check copies
            # mod.pbo -> addons/mod.pbo
            # mod.bikey -> keys/mod.bikey
            # readme.txt -> ignored
            
            copy_calls = mock_copy.call_args_list
            destinations = [call[0][1] for call in copy_calls]
            
            self.assertTrue(any("addons/mod.pbo" in d for d in destinations))
            self.assertTrue(any("keys/mod.bikey" in d for d in destinations))
            self.assertFalse(any("readme.txt" in d for d in destinations))

    @patch("os.remove")
    @patch("json.load")
    @patch("json.dump")
    @patch("os.path.exists")
    @patch("builtins.open", new_callable=mock_open)
    def test_sync_mods_cleanup(self, mock_file, mock_exists, mock_dump, mock_load, mock_remove):
        # Scenario: Mod 999 was installed, but is no longer in the list.
        # Lock file has entry for 999.
        
        mock_load.return_value = {
            "999": ["addons/old_mod.pbo"]
        }
        
        # Assume paths exist so cleanup tries to run
        mock_exists.return_value = True
        
        # Run sync with EMPTY mod list
        manage_mods.sync_mods(set())
        
        # Verify removal
        mock_remove.assert_called_with("addons/old_mod.pbo")

if __name__ == "__main__":
    unittest.main()
