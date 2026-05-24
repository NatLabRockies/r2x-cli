import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
COMPARE_SCRIPT = REPO_ROOT / "scripts" / "compare_benchmark_summary.py"


class CompareBenchmarkSummaryTests(unittest.TestCase):
    def test_compares_baseline_and_current(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            baseline.write_text(
                "\n".join(
                    [
                        "Benchmark r2x_reeds.parser: runs=3 total=30ms avg=10ms",
                        "Benchmark r2x_reeds.parser breakdown: python=7ms serialization=2ms (samples=3)",
                    ]
                )
            )
            current.write_text(
                "\n".join(
                    [
                        "Benchmark r2x_reeds.parser: runs=3 total=36ms avg=12ms",
                        "Benchmark r2x_reeds.parser breakdown: python=8ms serialization=3ms (samples=3)",
                    ]
                )
            )

            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                    "--baseline-run-id",
                    "123456789",
                    "--baseline-run-url",
                    "https://github.com/example/repo/actions/runs/123456789",
                ],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("### Plugin Benchmark Delta", result.stdout)
        self.assertIn(
            "Baseline run: [123456789](https://github.com/example/repo/actions/runs/123456789)",
            result.stdout,
        )
        self.assertIn("Summary: 1 regressed, 0 improved, 0 unchanged, 0 unknown.", result.stdout)
        self.assertIn(
            "| r2x_reeds.parser | 10ms | 12ms | +2.00ms | +20.00% | +1.00ms | +1.00ms |",
            result.stdout,
        )

    def test_no_overlapping_plugins_message(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            baseline.write_text("Benchmark old.plugin: runs=1 total=10ms avg=10ms\n")
            current.write_text("Benchmark new.plugin: runs=1 total=8ms avg=8ms\n")

            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                ],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("No overlapping benchmark plugins found.", result.stdout)

    def test_appends_to_github_step_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            summary = Path(tmp) / "step_summary.md"
            baseline.write_text("Benchmark r2x_reeds.parser: runs=1 total=10ms avg=10ms\n")
            current.write_text("Benchmark r2x_reeds.parser: runs=1 total=9ms avg=9ms\n")
            summary.write_text("## Existing\n")

            env = os.environ.copy()
            env["GITHUB_STEP_SUMMARY"] = str(summary)
            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                    "--append-summary",
                ],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            summary_text = summary.read_text()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("## Existing", summary_text)
        self.assertIn("### Plugin Benchmark Delta", summary_text)
        self.assertIn("| r2x_reeds.parser | 10ms | 9ms | -1.00ms | -10.00% |", summary_text)

    def test_rejects_missing_baseline_file(self):
        with tempfile.TemporaryDirectory() as tmp:
            current = Path(tmp) / "current.txt"
            current.write_text("Benchmark r2x_reeds.parser: runs=1 total=10ms avg=10ms\n")
            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    "/tmp/does-not-exist-r2x-baseline.txt",
                    "--current",
                    str(current),
                ],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("baseline benchmark file not found", result.stderr)

    def test_fails_when_regression_exceeds_threshold(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            baseline.write_text("Benchmark r2x_reeds.parser: runs=1 total=10ms avg=10ms\n")
            current.write_text("Benchmark r2x_reeds.parser: runs=1 total=12ms avg=12ms\n")

            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                    "--fail-on-regression-pct",
                    "10",
                ],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 2)
        self.assertIn("benchmark regressions above 10.00%", result.stderr)
        self.assertIn("r2x_reeds.parser (+20.00%)", result.stderr)

    def test_prints_machine_readable_status_line(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            baseline.write_text("Benchmark r2x_reeds.parser: runs=1 total=10ms avg=10ms\n")
            current.write_text("Benchmark r2x_reeds.parser: runs=1 total=9ms avg=9ms\n")

            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                    "--print-status-line",
                ],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 0)
        self.assertIn("BENCHMARK_DELTA_STATUS", result.stderr)
        self.assertIn("status=ok", result.stderr)
        self.assertIn("plugins=1", result.stderr)
        self.assertIn("threshold=n/a", result.stderr)

    def test_prints_no_data_status_line_when_no_overlap(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            baseline.write_text("Benchmark old.plugin: runs=1 total=10ms avg=10ms\n")
            current.write_text("Benchmark new.plugin: runs=1 total=9ms avg=9ms\n")

            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                    "--print-status-line",
                ],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 0)
        self.assertIn("BENCHMARK_DELTA_STATUS", result.stderr)
        self.assertIn("status=no_data", result.stderr)
        self.assertIn("plugins=0", result.stderr)

    def test_prints_fail_status_line_when_threshold_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            baseline.write_text("Benchmark r2x_reeds.parser: runs=1 total=10ms avg=10ms\n")
            current.write_text("Benchmark r2x_reeds.parser: runs=1 total=12ms avg=12ms\n")

            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                    "--fail-on-regression-pct",
                    "10",
                    "--print-status-line",
                ],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 2)
        self.assertIn("BENCHMARK_DELTA_STATUS", result.stderr)
        self.assertIn("status=fail", result.stderr)
        self.assertIn("threshold=10.00", result.stderr)
        self.assertIn("threshold_failures=1", result.stderr)

    def test_writes_github_output_status_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            gh_output = Path(tmp) / "github_output.txt"
            baseline.write_text("Benchmark r2x_reeds.parser: runs=1 total=10ms avg=10ms\n")
            current.write_text("Benchmark r2x_reeds.parser: runs=1 total=9ms avg=9ms\n")
            gh_output.write_text("")

            env = os.environ.copy()
            env["GITHUB_OUTPUT"] = str(gh_output)
            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                    "--write-github-output",
                ],
                cwd=REPO_ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            output_text = gh_output.read_text()

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("benchmark_delta_status=ok", output_text)
        self.assertIn("benchmark_delta_plugins=1", output_text)
        self.assertIn("benchmark_delta_regressed=0", output_text)
        self.assertIn("benchmark_delta_improved=1", output_text)
        self.assertIn("benchmark_delta_threshold=n/a", output_text)

    def test_rejects_negative_regression_threshold(self):
        with tempfile.TemporaryDirectory() as tmp:
            baseline = Path(tmp) / "baseline.txt"
            current = Path(tmp) / "current.txt"
            baseline.write_text("Benchmark r2x_reeds.parser: runs=1 total=10ms avg=10ms\n")
            current.write_text("Benchmark r2x_reeds.parser: runs=1 total=9ms avg=9ms\n")

            result = subprocess.run(
                [
                    "python3",
                    str(COMPARE_SCRIPT),
                    "--baseline",
                    str(baseline),
                    "--current",
                    str(current),
                    "--fail-on-regression-pct",
                    "-1",
                ],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 1)
        self.assertIn("--fail-on-regression-pct must be >= 0", result.stderr)


if __name__ == "__main__":
    unittest.main()
