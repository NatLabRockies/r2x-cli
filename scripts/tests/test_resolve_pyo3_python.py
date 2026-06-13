import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
RESOLVE_SCRIPT = REPO_ROOT / "scripts" / "resolve_pyo3_python.sh"


def write_executable(path: Path, content: str) -> None:
    path.write_text(content)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class ResolvePyo3PythonTests(unittest.TestCase):
    def test_uses_explicit_pyo3_python_when_executable(self):
        with tempfile.TemporaryDirectory() as tmp:
            python = Path(tmp) / "python"
            write_executable(
                python,
                """#!/usr/bin/env bash
if [ "$1" = "-c" ]; then
  echo 3.13
  exit 0
fi
exit 0
""",
            )

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            result = subprocess.run(
                [str(RESOLVE_SCRIPT)],
                check=True,
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.stdout.strip(), str(python))

    def test_explicit_pyo3_python_rejects_unsupported_interpreter(self):
        with tempfile.TemporaryDirectory() as tmp:
            python = Path(tmp) / "python"
            write_executable(
                python,
                """#!/usr/bin/env bash
if [ "$1" = "-c" ]; then
  echo 3.10
  exit 0
fi
exit 0
""",
            )

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            result = subprocess.run(
                [str(RESOLVE_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires Python 3.11 or newer", result.stderr)

    def test_explicit_pyo3_python_must_match_requested_python_abi(self):
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
                [str(RESOLVE_SCRIPT)],
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

    def test_explicit_pyo3_python_accepts_requested_patch_version_with_same_abi(self):
        with tempfile.TemporaryDirectory() as tmp:
            python = Path(tmp) / "python"
            write_executable(
                python,
                """#!/usr/bin/env bash
if [ "$1" = "-c" ]; then
  echo 3.13
  exit 0
fi
exit 0
""",
            )

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            env["R2X_PYTHON_VERSION"] = "3.13.1"
            result = subprocess.run(
                [str(RESOLVE_SCRIPT)],
                check=True,
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.stdout.strip(), str(python))

    def test_requested_version_must_resolve_exactly(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir()
            uv = bin_dir / "uv"
            write_executable(
                uv,
                """#!/usr/bin/env bash
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.13" ]; then
  echo /uv/python/3.13/bin/python3.13
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
                [str(RESOLVE_SCRIPT)],
                check=True,
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.stdout.strip(), "/uv/python/3.13/bin/python3.13")

    def test_requested_version_does_not_fallback_to_other_versions(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir()
            uv = bin_dir / "uv"
            write_executable(
                uv,
                """#!/usr/bin/env bash
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.12" ]; then
  echo /uv/python/3.12/bin/python3.12
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
                [str(RESOLVE_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("uv python install 3.13", result.stderr)
        self.assertIn("uv python find 3.13", result.stderr)

    def test_requested_patch_version_falls_back_to_requested_abi(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir()
            uv = bin_dir / "uv"
            write_executable(
                uv,
                """#!/usr/bin/env bash
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.13.1" ]; then
  exit 1
fi
if [ "$1" = "python" ] && [ "$2" = "find" ] && [ "$3" = "3.13" ]; then
  echo /uv/python/3.13/bin/python3.13
  exit 0
fi
exit 1
""",
            )

            env = os.environ.copy()
            env.pop("PYO3_PYTHON", None)
            env["R2X_PYTHON_VERSION"] = "3.13.1"
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            result = subprocess.run(
                [str(RESOLVE_SCRIPT)],
                check=True,
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.stdout.strip(), "/uv/python/3.13/bin/python3.13")

    def test_requested_patch_version_reports_fallback_hints_when_unavailable(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir()
            uv = bin_dir / "uv"
            write_executable(
                uv,
                """#!/usr/bin/env bash
exit 1
""",
            )

            env = os.environ.copy()
            env.pop("PYO3_PYTHON", None)
            env["R2X_PYTHON_VERSION"] = "3.13.1"
            env["PATH"] = f"{bin_dir}:{env['PATH']}"
            result = subprocess.run(
                [str(RESOLVE_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "Install it with: uv python install 3.13.1 || uv python install 3.13",
            result.stderr,
        )
        self.assertIn(
            "Verify with: uv python find 3.13.1 || uv python find 3.13",
            result.stderr,
        )

    def test_requested_version_rejects_unsupported_python_before_uv_lookup(self):
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
                [str(RESOLVE_SCRIPT)],
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

    def test_default_patch_version_reports_fallback_hints_when_no_python_found(self):
        with tempfile.TemporaryDirectory() as tmp:
            bin_dir = Path(tmp) / "bin"
            bin_dir.mkdir()
            bash_path = subprocess.run(
                ["which", "bash"],
                check=True,
                stdout=subprocess.PIPE,
                text=True,
            ).stdout.strip()
            dirname_path = subprocess.run(
                ["which", "dirname"],
                check=True,
                stdout=subprocess.PIPE,
                text=True,
            ).stdout.strip()
            (bin_dir / "bash").symlink_to(bash_path)
            (bin_dir / "dirname").symlink_to(dirname_path)

            env = os.environ.copy()
            env.pop("PYO3_PYTHON", None)
            env.pop("R2X_PYTHON_VERSION", None)
            env["R2X_DEFAULT_PYTHON_VERSION"] = "3.13.1"
            env["PATH"] = str(bin_dir)
            result = subprocess.run(
                [str(RESOLVE_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "Install uv and run: uv python install 3.13.1 || uv python install 3.13",
            result.stderr,
        )
        self.assertIn(
            "Verify with: uv python find 3.13.1 || uv python find 3.13",
            result.stderr,
        )


if __name__ == "__main__":
    unittest.main()
