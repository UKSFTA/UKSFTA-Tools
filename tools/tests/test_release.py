import unittest
import os
import sys
import shutil
from unittest.mock import patch, MagicMock, mock_open

sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import release

class TestReleaseTool(unittest.TestCase):

    def setUp(self):
        # Create a dummy script_version.hpp for testing
        self.test_version_file = "test_script_version.hpp"
        with open(self.test_version_file, "w") as f:
            f.write("#define MAJOR 1\n#define MINOR 0\n#define PATCHLVL 0\n#define BUILD 0")
        
        # Override the VERSION_FILE path in the module
        self.original_version_file = release.VERSION_FILE
        release.VERSION_FILE = self.test_version_file

    def tearDown(self):
        if os.path.exists(self.test_version_file):
            os.remove(self.test_version_file)
        release.VERSION_FILE = self.original_version_file

    def test_get_current_version(self):
        v_str, v_tuple = release.get_current_version()
        self.assertEqual(v_str, "1.0.0")
        self.assertEqual(v_tuple, (1, 0, 0))

    def test_bump_version_patch(self):
        new_v = release.bump_version("patch")
        self.assertEqual(new_v, "1.0.1")
        v_str, _ = release.get_current_version()
        self.assertEqual(v_str, "1.0.1")

    def test_bump_version_minor(self):
        release.bump_version("minor")
        v_str, _ = release.get_current_version()
        self.assertEqual(v_str, "1.1.0")

    def test_bump_version_major(self):
        release.bump_version("major")
        v_str, _ = release.get_current_version()
        self.assertEqual(v_str, "2.0.0")

    @patch("builtins.open", new_callable=mock_open)
    def test_create_vdf(self, mock_file):
        # Just check if file open is called with correct path and content structure
        release.HEMTT_OUT = "."
        vdf_path = release.create_vdf("123", "456", "/path/to/content", "changelog text")
        
        mock_file.assert_called()
        handle = mock_file()
        handle.write.assert_called()
        written_content = handle.write.call_args[0][0]
        
        self.assertIn('"appid" "123"', written_content)
        self.assertIn('"publishedfileid" "456"', written_content)
        self.assertIn('"contentfolder" "/path/to/content"', written_content)
        self.assertIn('"changenote" "changelog text"', written_content)

if __name__ == "__main__":
    unittest.main()
