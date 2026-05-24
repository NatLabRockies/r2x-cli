import unittest
from io import StringIO
from pathlib import Path
from unittest.mock import patch

from detect_uv_python import choose_uv_prefix, main, validate_python_version


class ChooseUvPrefixTests(unittest.TestCase):
    def test_prefers_uv_cache_paths(self):
        entries = [
            {"path": "/usr/local/bin/python3.12"},
            {
                "path": "/Users/test/.local/share/uv/python/cpython-3.12.5-macos-aarch64-none/bin/python3.12",
            },
        ]
        prefix = choose_uv_prefix(entries)
        self.assertEqual(
            prefix,
            Path("/Users/test/.local/share/uv/python/cpython-3.12.5-macos-aarch64-none"),
        )

    def test_fallback_to_first_entry(self):
        entries = [{"path": "/opt/homebrew/bin/python3.12"}]
        prefix = choose_uv_prefix(entries)
        self.assertEqual(prefix, Path("/opt/homebrew"))

    def test_main_prefers_r2x_python_version_env(self):
        entries = [{"path": "/uv/python/cpython-3.13.1-linux-x86_64-gnu/bin/python3.13"}]
        with (
            patch.dict("os.environ", {"R2X_PYTHON_VERSION": "3.13", "PY_VERSION": "3.12"}),
            patch("detect_uv_python.load_uv_python_list", return_value=entries) as load,
            patch("sys.stdout", new_callable=StringIO),
        ):
            result = main([])

        self.assertEqual(result, 0)
        load.assert_called_once_with("3.13")

    def test_validate_python_version_accepts_patch_version(self):
        self.assertEqual(validate_python_version(" 3.13.1 "), "3.13.1")

    def test_main_rejects_unsupported_python_version_before_uv_lookup(self):
        with (
            patch.dict("os.environ", {"R2X_PYTHON_VERSION": "3.10"}),
            patch("detect_uv_python.load_uv_python_list") as load,
            patch("sys.stderr", new_callable=StringIO) as stderr,
        ):
            result = main([])

        self.assertEqual(result, 1)
        self.assertIn("requires Python 3.11 or newer", stderr.getvalue())
        load.assert_not_called()


if __name__ == "__main__":
    unittest.main()
