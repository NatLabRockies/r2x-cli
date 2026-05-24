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
    def test_readme_documents_r2x_python_version_for_direct_cargo_builds(self):
        readme = (REPO_ROOT / "README.md").read_text()

        self.assertIn(
            "R2X_PYTHON_VERSION=3.12 cargo install --path crates/r2x-cli --force --locked",
            readme,
        )
        self.assertIn(
            "R2X_PYTHON_VERSION=3.13 cargo install --path crates/r2x-cli --force --locked",
            readme,
        )
        self.assertIn("R2X_PYTHON_VERSION=3.12 cargo build --release", readme)
        self.assertIn(
            "`cargo` now resolves `R2X_PYTHON_VERSION` through `uv python find` automatically.",
            readme,
        )

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
        self.assertIn("Resolved query", action)
        self.assertIn('if [ "$PYTHON_ABI_VERSION" != "$REQUESTED_PYTHON_ABI" ]; then', action)
        self.assertIn("python-version requested ABI", action)
        self.assertNotIn('REQUESTED_PYTHON_VERSION" =~', action)
        self.assertIn("R2X_PYTHON_VERSION=$REQUESTED_PYTHON_VERSION", action)
        self.assertNotIn("R2X_PYTHON_VERSION=$PYTHON_ABI_VERSION", action)
        self.assertIn('uv python install "$REQUESTED_PYTHON_VERSION"', action)
        self.assertIn('if ! uv python install "$REQUESTED_PYTHON_VERSION"; then', action)
        self.assertIn('uv python install "$REQUESTED_PYTHON_ABI"', action)
        self.assertIn('INSTALL_HINT="uv python install $REQUESTED_PYTHON_VERSION"', action)
        self.assertIn('FIND_HINT="uv python find $REQUESTED_PYTHON_VERSION"', action)
        self.assertIn('INSTALL_HINT="$INSTALL_HINT || uv python install $REQUESTED_PYTHON_ABI"', action)
        self.assertIn('FIND_HINT="$FIND_HINT || uv python find $REQUESTED_PYTHON_ABI"', action)
        self.assertIn("unable to install requested Python version", action)
        self.assertIn('UV_PYTHON_QUERY="$REQUESTED_PYTHON_VERSION"', action)
        self.assertIn('uv python find "$UV_PYTHON_QUERY"', action)
        self.assertIn('if [ -z "$UV_PYTHON_BIN" ] && [ "$REQUESTED_PYTHON_ABI" != "$REQUESTED_PYTHON_VERSION" ]; then', action)
        self.assertIn('UV_PYTHON_QUERY="$REQUESTED_PYTHON_ABI"', action)
        self.assertIn("unable to resolve Python for requested version", action)
        self.assertIn("Verify with: $FIND_HINT", action)
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
        self.assertIn("Benchmark fixture (parser repeat)", build)
        self.assertIn("R2X_BENCHMARK_SUMMARY_PATH", build)
        self.assertIn("test_run_plugin_benchmark_repeat_outputs_summary", build)
        self.assertIn("scripts.tests.test_format_benchmark_summary", build)
        self.assertIn("scripts.tests.test_compare_benchmark_summary", build)
        self.assertIn("Format benchmark summary table", build)
        self.assertIn("scripts/format_benchmark_summary.py", build)
        self.assertIn("Download baseline benchmark artifact", build)
        self.assertIn("Compare benchmark against baseline", build)
        self.assertIn("scripts/compare_benchmark_summary.py", build)
        self.assertIn("BASELINE_BENCHMARK_PATH", build)
        self.assertIn("BASELINE_BENCHMARK_RUN_ID", build)
        self.assertIn("BASELINE_BENCHMARK_RUN_URL", build)
        self.assertIn("No baseline artifact found from recent successful", build)
        self.assertIn("workflow_runs[]", build)
        self.assertIn("--baseline-run-id", build)
        self.assertIn("--baseline-run-url", build)
        self.assertIn("R2X_BENCHMARK_REGRESSION_PCT", build)
        self.assertIn("--fail-on-regression-pct", build)
        self.assertIn("--print-status-line", build)
        self.assertIn("--write-github-output", build)
        self.assertIn("r2x-plugin-benchmark-summary", build)
        self.assertIn("r2x-plugin-benchmark.md", build)
        self.assertIn("r2x-plugin-benchmark-delta.md", build)
        self.assertIn("actions/upload-artifact@v4", build)
        self.assertIn("r2x-plugin-benchmark-summary", build)

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
        self.assertIn(
            "PyO3 remediation: Install and use a supported interpreter (Python 3.11 or newer) for PYO3_PYTHON",
            result.stdout,
        )
        self.assertIn("| status | `error` |", summary_text)
        self.assertIn("requires Python 3.11 or newer", summary_text)
        self.assertIn("| remediation | `Install and use a supported interpreter", summary_text)

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
        self.assertIn(
            "PyO3 remediation: Set PYO3_PYTHON to a valid interpreter path",
            result.stdout,
        )

    def test_diagnostics_unset_pyo3_python_with_patch_request_suggests_abi_fallback(self):
        env = os.environ.copy()
        env.pop("PYO3_PYTHON", None)
        env["R2X_PYTHON_VERSION"] = "3.13.1"
        result = subprocess.run(
            ["bash", str(DIAGNOSTICS_SCRIPT)],
            cwd=REPO_ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn(
            "PyO3 remediation: Set PYO3_PYTHON to a valid interpreter path (example: uv python find 3.13.1 || uv python find 3.13)",
            result.stdout,
        )

    def test_diagnostics_non_executable_pyo3_python_with_patch_request_suggests_abi_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            python = Path(tmp) / "python"
            python.write_text("#!/usr/bin/env bash\nexit 0\n")
            python.chmod(stat.S_IRUSR | stat.S_IWUSR)

            env = os.environ.copy()
            env["PYO3_PYTHON"] = str(python)
            env["R2X_PYTHON_VERSION"] = "3.13.1"
            result = subprocess.run(
                ["bash", str(DIAGNOSTICS_SCRIPT)],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("PYO3_PYTHON is not executable", result.stdout)
        self.assertIn(
            "PyO3 remediation: Install Python with uv python install 3.13.1 || uv python install 3.13, then reset PYO3_PYTHON (example: uv python find 3.13.1 || uv python find 3.13)",
            result.stdout,
        )

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
        self.assertIn(
            "PyO3 remediation: Align them by setting PYO3_PYTHON to uv python find 3.13.1 || uv python find 3.13",
            result.stdout,
        )
        self.assertIn("| requested Python ABI | `3.13` |", summary_text)
        self.assertIn("| status | `error` |", summary_text)
        self.assertIn(
            "| remediation | `Align them by setting PYO3_PYTHON to uv python find 3.13.1 || uv python find 3.13` |",
            summary_text,
        )


if __name__ == "__main__":
    unittest.main()
