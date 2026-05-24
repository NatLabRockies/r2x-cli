import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
VERSION_SCRIPT = REPO_ROOT / "scripts" / "python_version.sh"


def run_bash(command: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", "-c", command],
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


class PythonVersionShellTests(unittest.TestCase):
    def test_python_abi_version_accepts_patch_version(self):
        result = run_bash(
            f'source "{VERSION_SCRIPT}"; r2x_python_abi_version R2X_PYTHON_VERSION 3.13.1'
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "3.13")

    def test_python_abi_version_rejects_unsupported_version(self):
        result = run_bash(
            f'source "{VERSION_SCRIPT}"; r2x_python_abi_version R2X_PYTHON_VERSION 3.10'
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires Python 3.11 or newer", result.stderr)

    def test_validate_python_version_rejects_malformed_version(self):
        result = run_bash(
            f'source "{VERSION_SCRIPT}"; r2x_validate_python_version R2X_PYTHON_VERSION 3.13-dev'
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must be a Python version like 3.12 or 3.12.1", result.stderr)

    def test_python_install_hint_for_patch_version_includes_abi_fallback(self):
        result = run_bash(
            f'source "{VERSION_SCRIPT}"; r2x_python_install_hint 3.13.1'
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.strip(),
            "uv python install 3.13.1 || uv python install 3.13",
        )

    def test_python_find_hint_for_patch_version_includes_abi_fallback(self):
        result = run_bash(
            f'source "{VERSION_SCRIPT}"; r2x_python_find_hint 3.13.1'
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            result.stdout.strip(),
            "uv python find 3.13.1 || uv python find 3.13",
        )

    def test_python_hints_for_major_minor_version_use_single_command(self):
        install = run_bash(
            f'source "{VERSION_SCRIPT}"; r2x_python_install_hint 3.13'
        )
        find = run_bash(
            f'source "{VERSION_SCRIPT}"; r2x_python_find_hint 3.13'
        )

        self.assertEqual(install.returncode, 0, install.stderr)
        self.assertEqual(find.returncode, 0, find.stderr)
        self.assertEqual(install.stdout.strip(), "uv python install 3.13")
        self.assertEqual(find.stdout.strip(), "uv python find 3.13")

    def test_find_uv_python_falls_back_to_patch_abi(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir(parents=True, exist_ok=True)
            uv = bin_dir / "uv"
            uv.write_text(
                """#!/usr/bin/env bash
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.13.1" ]; then
  exit 1
fi
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.13" ]; then
  echo /uv/python/3.13/bin/python3.13
  exit 0
fi
exit 1
"""
            )
            uv.chmod(0o755)

            env = dict(os.environ)
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            result = subprocess.run(
                [
                    "bash",
                    "-c",
                    f'source "{VERSION_SCRIPT}"; r2x_find_uv_python 3.13.1',
                ],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "/uv/python/3.13/bin/python3.13")

    def test_find_uv_python_returns_nonzero_when_no_match(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir(parents=True, exist_ok=True)
            uv = bin_dir / "uv"
            uv.write_text("#!/usr/bin/env bash\nexit 1\n")
            uv.chmod(0o755)

            env = dict(os.environ)
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            result = subprocess.run(
                [
                    "bash",
                    "-c",
                    f'source "{VERSION_SCRIPT}"; r2x_find_uv_python 3.13.1',
                ],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)

    def test_find_uv_python_does_not_fallback_for_major_minor_request(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir(parents=True, exist_ok=True)
            uv = bin_dir / "uv"
            uv.write_text(
                """#!/usr/bin/env bash
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.13" ]; then
  exit 1
fi
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.12" ]; then
  echo /uv/python/3.12/bin/python3.12
  exit 0
fi
exit 1
"""
            )
            uv.chmod(0o755)

            env = dict(os.environ)
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            result = subprocess.run(
                [
                    "bash",
                    "-c",
                    f'source "{VERSION_SCRIPT}"; r2x_find_uv_python 3.13',
                ],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
