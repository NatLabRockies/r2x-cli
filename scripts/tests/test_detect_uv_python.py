import unittest
from pathlib import Path

from detect_uv_python import choose_uv_prefix


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


if __name__ == "__main__":
    unittest.main()
