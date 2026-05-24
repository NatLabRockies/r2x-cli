import os
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
FORMAT_SCRIPT = REPO_ROOT / "scripts" / "format_benchmark_summary.py"


class FormatBenchmarkSummaryTests(unittest.TestCase):
    def test_formats_summary_and_breakdown_rows(self):
        with tempfile.TemporaryDirectory() as tmp:
            bench = Path(tmp) / "benchmark.txt"
            bench.write_text(
                "\n".join(
                    [
                        "warning: prelude line",
                        "Benchmark r2x_reeds.parser: runs=3 total=12ms avg=4ms",
                        "Benchmark r2x_reeds.parser breakdown: python=3ms serialization=1ms (samples=3)",
                    ]
                )
            )

            result = subprocess.run(
                ["python3", str(FORMAT_SCRIPT), "--input", str(bench)],
                cwd=REPO_ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("### Plugin Benchmark Summary", result.stdout)
        self.assertIn("| r2x_reeds.parser | 3 | 12ms | 4ms | 3ms | 1ms | 3 |", result.stdout)

    def test_appends_to_github_step_summary(self):
        with tempfile.TemporaryDirectory() as tmp:
            bench = Path(tmp) / "benchmark.txt"
            summary = Path(tmp) / "step_summary.md"
            bench.write_text("Benchmark r2x_reeds.parser: runs=2 total=10ms avg=5ms\n")
            summary.write_text("## Existing\n")

            env = os.environ.copy()
            env["GITHUB_STEP_SUMMARY"] = str(summary)
            result = subprocess.run(
                [
                    "python3",
                    str(FORMAT_SCRIPT),
                    "--input",
                    str(bench),
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
        self.assertIn("### Plugin Benchmark Summary", summary_text)
        self.assertIn("| r2x_reeds.parser | 2 | 10ms | 5ms | n/a | n/a | n/a |", summary_text)

    def test_rejects_missing_input_file(self):
        result = subprocess.run(
            ["python3", str(FORMAT_SCRIPT), "--input", "/tmp/does-not-exist-r2x-bench.txt"],
            cwd=REPO_ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("benchmark input file not found", result.stderr)


if __name__ == "__main__":
    unittest.main()
