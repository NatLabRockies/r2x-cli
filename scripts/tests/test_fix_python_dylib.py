import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
FIX_SCRIPT = REPO_ROOT / "scripts" / "fix_python_dylib.sh"


def run_bash(command: str) -> str:
    result = subprocess.run(
        ["bash", "-c", command],
        check=True,
        cwd=REPO_ROOT,
        stdout=subprocess.PIPE,
        text=True,
    )
    return result.stdout.strip()


def write_executable(path: Path, content: str) -> None:
    path.write_text(content)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class FixPythonDylibTests(unittest.TestCase):
    def test_python_abi_version_strips_patch_component(self):
        output = run_bash(f'source "{FIX_SCRIPT}"; python_abi_version 3.13.1')

        self.assertEqual(output, "3.13")

    def test_resolve_python_version_normalizes_configured_version(self):
        output = run_bash(
            f'source "{FIX_SCRIPT}"; R2X_PYTHON_VERSION=3.13.1 resolve_python_version'
        )

        self.assertEqual(output, "3.13")

    def test_resolve_python_version_rejects_unsupported_configured_version(self):
        result = subprocess.run(
            [
                "bash",
                "-c",
                f'source "{FIX_SCRIPT}"; R2X_PYTHON_VERSION=3.10 resolve_python_version',
            ],
            cwd=REPO_ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires Python 3.11 or newer", result.stderr)

    def test_resolve_python_version_rejects_pyo3_mismatch_with_configured_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            python = Path(tmp) / "python"
            write_executable(
                python,
                """#!/usr/bin/env bash
if [ "$1" = "-c" ]; then
  echo 3.12
  exit 0
fi
exit 0
""",
            )

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            env["R2X_PYTHON_VERSION"] = "3.13.1"
            result = subprocess.run(
                ["bash", "-c", f'source "{FIX_SCRIPT}"; resolve_python_version'],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "PYO3_PYTHON resolves to Python 3.12 but R2X_PYTHON_VERSION requests 3.13",
            result.stderr,
        )

    def test_resolve_uv_python_tag_does_not_fallback_when_version_configured(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir()
            uv = bin_dir / "uv"
            write_executable(
                uv,
                """#!/usr/bin/env bash
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.12" ]; then
  echo /uv/python/cpython-3.12-linux-x86_64-gnu/bin/python3.12
  exit 0
fi
exit 1
""",
            )

            env = os.environ.copy()
            env.pop("PYO3_PYTHON", None)
            env["R2X_PYTHON_VERSION"] = "3.13"
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            result = subprocess.run(
                ["bash", "-c", f'source "{FIX_SCRIPT}"; resolve_uv_python_tag'],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "")

    def test_resolve_uv_python_tag_rejects_unsupported_configured_version_before_uv_lookup(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir()
            uv = bin_dir / "uv"
            uv_marker = Path(tmp) / "uv-was-called"
            write_executable(
                uv,
                f"""#!/usr/bin/env bash
touch "{uv_marker}"
exit 1
""",
            )

            env = os.environ.copy()
            env.pop("PYO3_PYTHON", None)
            env["R2X_PYTHON_VERSION"] = "3.10"
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            result = subprocess.run(
                ["bash", "-c", f'source "{FIX_SCRIPT}"; resolve_uv_python_tag'],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            uv_was_called = uv_marker.exists()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires Python 3.11 or newer", result.stderr)
        self.assertFalse(uv_was_called, "uv should not run for unsupported versions")

    def test_resolve_uv_python_tag_rejects_pyo3_mismatch_with_configured_version(self):
        with tempfile.TemporaryDirectory() as tmp:
            python_root = Path(tmp) / "cpython-3.12-linux-x86_64-gnu"
            bin_dir = python_root / "bin"
            bin_dir.mkdir(parents=True)
            python = bin_dir / "python3.12"
            write_executable(
                python,
                """#!/usr/bin/env bash
if [ "$1" = "-c" ]; then
  echo 3.12
  exit 0
fi
exit 0
""",
            )

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            env["R2X_PYTHON_VERSION"] = "3.13"
            result = subprocess.run(
                ["bash", "-c", f'source "{FIX_SCRIPT}"; resolve_uv_python_tag'],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout.strip(), "")
        self.assertIn("PYO3_PYTHON resolves to Python 3.12", result.stderr)

    def test_main_reports_usage_without_binary_argument(self):
        result = subprocess.run(
            [str(FIX_SCRIPT)],
            cwd=REPO_ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Usage:", result.stdout)


if __name__ == "__main__":
    unittest.main()
