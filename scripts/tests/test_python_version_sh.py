import subprocess
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


if __name__ == "__main__":
    unittest.main()
