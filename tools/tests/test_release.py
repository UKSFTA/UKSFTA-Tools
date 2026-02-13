import unittest
from unittest.mock import patch, mock_open, MagicMock
import os
import sys

# Add parent dir to path
sys.path.append(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import release

class TestReleaseTool(unittest.TestCase):

    def test_get_current_version(self):
        content = "#define MAJOR 1\n#define MINOR 2\n#define PATCHLVL 3"
        with patch("builtins.open", mock_open(read_data=content)):
            with patch("os.path.exists", return_value=True):
                # Ensure release uses a known mocked path for its global
                with patch("release.VERSION_FILE", "dummy_path.hpp"):
                    v_str, v_tuple = release.get_current_version()
                    self.assertEqual(v_str, "1.2.3")
                    self.assertEqual(v_tuple, (1, 2, 3))

    @patch("release.get_current_version")
    @patch("builtins.open", new_callable=mock_open, read_data="#define MAJOR 1\n#define MINOR 2\n#define PATCHLVL 3")
    def test_bump_version(self, mock_file, mock_version):
        mock_version.return_value = ("1.2.3", (1, 2, 3))
        # Ensure release uses a known mocked path
        with patch("release.VERSION_FILE", "dummy_path.hpp"):
            new_v = release.bump_version("patch")
            self.assertEqual(new_v, "1.2.4")

    @patch("os.makedirs")
    @patch("workshop_utils.resolve_transitive_dependencies")
    @patch("release.get_mod_categories")
    @patch("builtins.open", new_callable=mock_open)
    @patch("os.path.exists")
    def test_create_vdf(self, mock_exists, mock_file, mock_cats, mock_resolve, mock_mkdir):
        # Setup mocks
        mock_exists.return_value = True
        mock_cats.return_value = (
            [{"id": "123", "name": "Included Mod"}],
            {"123", "456"}
        )
        mock_resolve.return_value = {
            "123": {"name": "Included Mod", "dependencies": []},
            "789": {"name": "Transitive Mod", "dependencies": []}
        }
        
        # Run create_vdf
        vdf_path, desc_path = release.create_vdf("107410", "9999", "/tmp/content", "Changelog")
        
        self.assertIn("upload.vdf", vdf_path)
        self.assertIn("workshop_description_final.txt", desc_path)

if __name__ == "__main__":
    unittest.main()
