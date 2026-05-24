import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
PATCH_SCRIPT = REPO_ROOT / "scripts" / "patch_dist_installer.sh"


class PatchDistInstallerTests(unittest.TestCase):
    def test_patches_installer_with_configured_python_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            installer = Path(tmp) / "installer.sh"
            installer.write_text(
                "\n".join(
                    [
                        "#!/usr/bin/env bash",
                        "check_for_shadowed_bins() {",
                        "    :",
                        "}",
                        "install() {",
                        '    say "everything installed!"',
                        "}",
                        "",
                    ]
                )
            )

            env = os.environ.copy()
            env["R2X_PYTHON_VERSION"] = "3.13.1"
            subprocess.run(
                [str(PATCH_SCRIPT), str(installer)],
                check=True,
                cwd=REPO_ROOT,
                env=env,
            )

            patched = installer.read_text()
            self.assertIn('local _python_request_version="3.13.1"', patched)
            self.assertIn('local _python_abi_version="3.13"', patched)
            self.assertIn('local _primary_lib="libpython${_python_abi_version}.so.1.0"', patched)
            self.assertIn('local _primary_lib="libpython${_python_abi_version}.dylib"', patched)
            self.assertIn('uv python install "$_python_request_version"', patched)
            self.assertNotIn("libpython3.12.so", patched)
            self.assertNotIn("libpython3.13.1", patched)

    def test_rejects_unsupported_python_version_without_patching_installer(self):
        with tempfile.TemporaryDirectory() as tmp:
            installer = Path(tmp) / "installer.sh"
            original = "\n".join(
                [
                    "#!/usr/bin/env bash",
                    "check_for_shadowed_bins() {",
                    "    :",
                    "}",
                    "install() {",
                    '    say "everything installed!"',
                    "}",
                    "",
                ]
            )
            installer.write_text(original)

            env = os.environ.copy()
            env["R2X_PYTHON_VERSION"] = "3.10"
            result = subprocess.run(
                [str(PATCH_SCRIPT), str(installer)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            patched = installer.read_text()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires Python 3.11 or newer", result.stderr)
        self.assertEqual(patched, original)


if __name__ == "__main__":
    unittest.main()
