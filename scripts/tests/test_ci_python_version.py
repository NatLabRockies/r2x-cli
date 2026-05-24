import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
DIAGNOSTICS_SCRIPT = REPO_ROOT / "scripts" / "ci_pyo3_diagnostics.sh"


def write_executable(path: Path, content: str) -> None:
    path.write_text(content)
    path.chmod(path.stat().st_mode | stat.S_IXUSR)


class CiPythonVersionTests(unittest.TestCase):
    def test_release_workflow_uses_single_python_version_env(self):
        workflow = (REPO_ROOT / ".github" / "workflows" / "release.yml").read_text()

        self.assertIn('R2X_PYTHON_VERSION: "3.12"', workflow)
        self.assertIn("python-version: ${{ env.R2X_PYTHON_VERSION }}", workflow)
        self.assertNotIn('python-version: "3.12"', workflow)

    def test_build_setup_reference_uses_python_version_env(self):
        build_setup = (REPO_ROOT / ".github" / "build-setup.yml").read_text()

        self.assertIn("python-version: ${{ env.R2X_PYTHON_VERSION }}", build_setup)
        self.assertNotIn('python-version: "3.12"', build_setup)

    def test_setup_action_preserves_requested_version_and_exports_abi(self):
        action = (REPO_ROOT / ".github" / "actions" / "setup-uv-python" / "action.yml").read_text()

        self.assertIn("python-abi-version=$PYTHON_ABI_VERSION", action)
        self.assertIn('REQUESTED_PYTHON_VERSION="${{ inputs.python-version }}"', action)
        self.assertIn('source "${GITHUB_WORKSPACE:-$PWD}/scripts/python_version.sh"', action)
        self.assertIn('r2x_python_abi_version "python-version" "$REQUESTED_PYTHON_VERSION"', action)
        self.assertIn("Requested ABI", action)
        self.assertIn('if [ "$PYTHON_ABI_VERSION" != "$REQUESTED_PYTHON_ABI" ]; then', action)
        self.assertIn("python-version requested ABI", action)
        self.assertNotIn('REQUESTED_PYTHON_VERSION" =~', action)
        self.assertIn("R2X_PYTHON_VERSION=$REQUESTED_PYTHON_VERSION", action)
        self.assertNotIn("R2X_PYTHON_VERSION=$PYTHON_ABI_VERSION", action)
        self.assertIn('uv python install "$REQUESTED_PYTHON_VERSION"', action)
        self.assertIn('uv python find "$REQUESTED_PYTHON_VERSION"', action)
        self.assertIn("libpython${PYTHON_ABI_VERSION}.dylib", action)
        self.assertNotIn("libpython${{ inputs.python-version }}.dylib", action)

    def test_ci_workflows_emit_pyo3_diagnostics(self):
        build = (REPO_ROOT / ".github" / "workflows" / "build.yml").read_text()
        release = (REPO_ROOT / ".github" / "workflows" / "release.yml").read_text()
        build_setup = (REPO_ROOT / ".github" / "build-setup.yml").read_text()
        diagnostics = (REPO_ROOT / "scripts" / "ci_pyo3_diagnostics.sh").read_text()

        self.assertGreaterEqual(build.count("bash scripts/ci_pyo3_diagnostics.sh"), 2)
        self.assertIn("bash scripts/ci_pyo3_diagnostics.sh", release)
        self.assertIn("CARGO_BUILD_TARGET: ${{ join(matrix.targets, ' ') }}", release)
        self.assertIn("bash scripts/ci_pyo3_diagnostics.sh", build_setup)
        self.assertIn("PYO3_PYTHON", diagnostics)
        self.assertIn("PYO3_CONFIG_FILE", diagnostics)
        self.assertIn("PYO3_CROSS_LIB_DIR", diagnostics)
        self.assertIn("PYO3_CROSS_PYTHON_VERSION", diagnostics)
        self.assertIn("GITHUB_STEP_SUMMARY", diagnostics)
        self.assertIn("cargo target", diagnostics)
        self.assertNotIn("| target |", diagnostics)

    def test_diagnostics_accepts_supported_pyo3_python(self):
        with tempfile.TemporaryDirectory() as tmp:
            python = Path(tmp) / "python"
            summary = Path(tmp) / "summary.md"
            write_executable(
                python,
                """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then
  echo "Python 3.13.1"
  exit 0
fi
if [ "$1" = "-c" ]; then
  case "$2" in
    *"sys.executable"*) echo "$0" ;;
    *"sys.prefix"*) echo "/fake/python" ;;
    *"version_info.major"*) echo "3.13" ;;
    *"LIBDIR"*) echo "/fake/python/lib" ;;
  esac
  exit 0
fi
exit 1
""",
            )

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            env["PYO3_CROSS"] = "1"
            env["PYO3_CROSS_LIB_DIR"] = "/fake/cross/lib"
            env["PYO3_CROSS_PYTHON_VERSION"] = "3.13"
            env["R2X_PYTHON_VERSION"] = "3.13.1"
            env["GITHUB_STEP_SUMMARY"] = str(summary)
            result = subprocess.run(
                ["bash", str(DIAGNOSTICS_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            summary_text = summary.read_text()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("PyO3 Python:", result.stdout)
        self.assertIn("| status | `ok` |", summary_text)
        self.assertIn("| PYO3_CROSS | `1` |", summary_text)
        self.assertIn("| PYO3_CROSS_LIB_DIR | `/fake/cross/lib` |", summary_text)
        self.assertIn("| PYO3_CROSS_PYTHON_VERSION | `3.13` |", summary_text)
        self.assertIn("| requested Python ABI | `3.13` |", summary_text)
        self.assertIn("| Python ABI | `3.13` |", summary_text)

    def test_diagnostics_rejects_unsupported_pyo3_python(self):
        with tempfile.TemporaryDirectory() as tmp:
            python = Path(tmp) / "python"
            summary = Path(tmp) / "summary.md"
            write_executable(
                python,
                """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then
  echo "Python 3.10.13"
  exit 0
fi
if [ "$1" = "-c" ]; then
  case "$2" in
    *"version_info.major"*) echo "3.10" ;;
    *) echo "" ;;
  esac
  exit 0
fi
exit 1
""",
            )

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            env["GITHUB_STEP_SUMMARY"] = str(summary)
            result = subprocess.run(
                ["bash", str(DIAGNOSTICS_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            summary_text = summary.read_text()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requires Python 3.11 or newer", result.stdout)
        self.assertIn("| status | `error` |", summary_text)
        self.assertIn("requires Python 3.11 or newer", summary_text)

    def test_diagnostics_rejects_unset_pyo3_python(self):
        env = os.environ.copy()
        env.pop("PYO3_PYTHON", None)
        result = subprocess.run(
            ["bash", str(DIAGNOSTICS_SCRIPT)],
            cwd=REPO_ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("PYO3_PYTHON is unset", result.stdout)

    def test_diagnostics_rejects_requested_and_selected_python_mismatch(self):
        with tempfile.TemporaryDirectory() as tmp:
            python = Path(tmp) / "python"
            summary = Path(tmp) / "summary.md"
            write_executable(
                python,
                """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then
  echo "Python 3.12.10"
  exit 0
fi
if [ "$1" = "-c" ]; then
  case "$2" in
    *"sys.executable"*) echo "$0" ;;
    *"sys.prefix"*) echo "/fake/python" ;;
    *"version_info.major"*) echo "3.12" ;;
    *"LIBDIR"*) echo "/fake/python/lib" ;;
  esac
  exit 0
fi
exit 1
""",
            )

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            env["R2X_PYTHON_VERSION"] = "3.13.1"
            env["GITHUB_STEP_SUMMARY"] = str(summary)
            result = subprocess.run(
                ["bash", str(DIAGNOSTICS_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            summary_text = summary.read_text()

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("requests Python ABI 3.13", result.stdout)
        self.assertIn("PYO3_PYTHON reports 3.12", result.stdout)
        self.assertIn("| requested Python ABI | `3.13` |", summary_text)
        self.assertIn("| status | `error` |", summary_text)


if __name__ == "__main__":
    unittest.main()
