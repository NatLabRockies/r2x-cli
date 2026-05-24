#!/usr/bin/env python3
"""Compare benchmark output files and emit a markdown delta table."""

from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

from format_benchmark_summary import parse_benchmark_lines


def rows_by_plugin(content: str) -> dict[str, dict[str, str]]:
    rows = parse_benchmark_lines(content)
    return {row["plugin"]: row for row in rows if "plugin" in row}


def parse_duration_ms(value: str | None) -> float | None:
    if not value or value == "n/a":
        return None
    if value.endswith("ms"):
        try:
            return float(value[:-2])
        except ValueError:
            return None
    if value.endswith("s"):
        try:
            return float(value[:-1]) * 1000.0
        except ValueError:
            return None
    return None


def format_ms_delta(delta_ms: float | None) -> str:
    if delta_ms is None:
        return "n/a"
    return f"{delta_ms:+.2f}ms"


def format_percent_delta(old_ms: float | None, new_ms: float | None) -> str:
    if old_ms is None or new_ms is None or old_ms == 0:
        return "n/a"
    pct = ((new_ms - old_ms) / old_ms) * 100.0
    return f"{pct:+.2f}%"


def parse_percent(value: str | None) -> float | None:
    if not value or value == "n/a":
        return None
    if not value.endswith("%"):
        return None
    try:
        return float(value[:-1])
    except ValueError:
        return None


def compare_rows(
    baseline: dict[str, dict[str, str]],
    current: dict[str, dict[str, str]],
) -> list[dict[str, str]]:
    plugins = sorted(set(baseline) & set(current))
    rows: list[dict[str, str]] = []
    for plugin in plugins:
        base = baseline[plugin]
        now = current[plugin]
        base_avg = parse_duration_ms(base.get("avg"))
        now_avg = parse_duration_ms(now.get("avg"))
        base_py = parse_duration_ms(base.get("python"))
        now_py = parse_duration_ms(now.get("python"))
        base_ser = parse_duration_ms(base.get("serialization"))
        now_ser = parse_duration_ms(now.get("serialization"))

        rows.append(
            {
                "plugin": plugin,
                "baseline_avg": base.get("avg", "n/a"),
                "current_avg": now.get("avg", "n/a"),
                "avg_delta": format_ms_delta(
                    (now_avg - base_avg) if now_avg is not None and base_avg is not None else None
                ),
                "avg_delta_pct": format_percent_delta(base_avg, now_avg),
                "python_delta": format_ms_delta(
                    (now_py - base_py) if now_py is not None and base_py is not None else None
                ),
                "serialization_delta": format_ms_delta(
                    (now_ser - base_ser) if now_ser is not None and base_ser is not None else None
                ),
            }
        )

    return rows


def summarize_rows(rows: list[dict[str, str]]) -> str:
    regressed = 0
    improved = 0
    unchanged = 0
    unknown = 0

    for row in rows:
        pct = parse_percent(row.get("avg_delta_pct"))
        if pct is None:
            unknown += 1
        elif pct > 0:
            regressed += 1
        elif pct < 0:
            improved += 1
        else:
            unchanged += 1

    return (
        f"Summary: {regressed} regressed, {improved} improved, "
        f"{unchanged} unchanged, {unknown} unknown."
    )


def regressions_above_threshold(
    rows: list[dict[str, str]], threshold_pct: float
) -> list[dict[str, str]]:
    regressions: list[dict[str, str]] = []
    for row in rows:
        pct = parse_percent(row.get("avg_delta_pct"))
        if pct is not None and pct > threshold_pct:
            regressions.append(row)
    return regressions


def classify_rows(rows: list[dict[str, str]]) -> tuple[int, int, int, int]:
    regressed = 0
    improved = 0
    unchanged = 0
    unknown = 0

    for row in rows:
        pct = parse_percent(row.get("avg_delta_pct"))
        if pct is None:
            unknown += 1
        elif pct > 0:
            regressed += 1
        elif pct < 0:
            improved += 1
        else:
            unchanged += 1

    return regressed, improved, unchanged, unknown


def format_markdown(
    rows: list[dict[str, str]],
    *,
    baseline_run_id: str | None = None,
    baseline_run_url: str | None = None,
) -> str:
    context_lines: list[str] = []
    if baseline_run_id and baseline_run_url:
        context_lines.append(f"Baseline run: [{baseline_run_id}]({baseline_run_url})")
    elif baseline_run_id:
        context_lines.append(f"Baseline run: `{baseline_run_id}`")

    if not rows:
        output = ["### Plugin Benchmark Delta", ""]
        output.extend(context_lines)
        if context_lines:
            output.append("")
        output.append("No overlapping benchmark plugins found.")
        return "\n".join(output) + "\n"

    header = [
        "### Plugin Benchmark Delta",
        "",
    ]
    header.extend(context_lines)
    if context_lines:
        header.append("")
    header.append(summarize_rows(rows))
    header.append("")
    header.extend(
        [
            "| Plugin | Baseline Avg | Current Avg | Avg Delta | Avg Delta % | Python Delta | Serialization Delta |",
            "| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
        ]
    )
    body = [
        "| {plugin} | {baseline_avg} | {current_avg} | {avg_delta} | {avg_delta_pct} | {python_delta} | {serialization_delta} |".format(
            **row
        )
        for row in rows
    ]
    return "\n".join(header + body) + "\n"


def append_to_step_summary(markdown: str) -> None:
    summary_path = Path(os.environ["GITHUB_STEP_SUMMARY"])
    with summary_path.open("a", encoding="utf-8") as handle:
        handle.write("\n")
        handle.write(markdown)


def write_github_output(values: dict[str, str]) -> None:
    output_path = Path(os.environ["GITHUB_OUTPUT"])
    with output_path.open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={value}\n")


def build_status_line(
    rows: list[dict[str, str]],
    threshold_pct: float | None,
    regressions: list[dict[str, str]],
) -> str:
    regressed, improved, unchanged, unknown = classify_rows(rows)
    if not rows:
        status = "no_data"
    elif regressions:
        status = "fail"
    elif regressed > 0:
        status = "warn"
    else:
        status = "ok"

    threshold_part = (
        f" threshold={threshold_pct:.2f}" if threshold_pct is not None else " threshold=n/a"
    )
    return (
        "BENCHMARK_DELTA_STATUS"
        f" status={status}"
        f" plugins={len(rows)}"
        f" regressed={regressed}"
        f" improved={improved}"
        f" unchanged={unchanged}"
        f" unknown={unknown}"
        f"{threshold_part}"
        f" threshold_failures={len(regressions)}"
    )


def status_values(
    rows: list[dict[str, str]],
    threshold_pct: float | None,
    regressions: list[dict[str, str]],
) -> dict[str, str]:
    regressed, improved, unchanged, unknown = classify_rows(rows)
    if not rows:
        status = "no_data"
    elif regressions:
        status = "fail"
    elif regressed > 0:
        status = "warn"
    else:
        status = "ok"

    threshold_value = f"{threshold_pct:.2f}" if threshold_pct is not None else "n/a"
    return {
        "benchmark_delta_status": status,
        "benchmark_delta_plugins": str(len(rows)),
        "benchmark_delta_regressed": str(regressed),
        "benchmark_delta_improved": str(improved),
        "benchmark_delta_unchanged": str(unchanged),
        "benchmark_delta_unknown": str(unknown),
        "benchmark_delta_threshold": threshold_value,
        "benchmark_delta_threshold_failures": str(len(regressions)),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, help="Path to baseline benchmark text")
    parser.add_argument("--current", required=True, help="Path to current benchmark text")
    parser.add_argument("--baseline-run-id", help="Optional baseline workflow run ID")
    parser.add_argument("--baseline-run-url", help="Optional baseline workflow run URL")
    parser.add_argument(
        "--fail-on-regression-pct",
        type=float,
        help="Fail if any plugin avg regression exceeds this positive percent",
    )
    parser.add_argument("--output", help="Optional output markdown path")
    parser.add_argument(
        "--print-status-line",
        action="store_true",
        help="Print a machine-readable BENCHMARK_DELTA_STATUS line to stderr",
    )
    parser.add_argument(
        "--write-github-output",
        action="store_true",
        help="Write benchmark delta status keys to GITHUB_OUTPUT when available",
    )
    parser.add_argument(
        "--append-summary",
        action="store_true",
        help="Append markdown output to GITHUB_STEP_SUMMARY when available",
    )
    args = parser.parse_args(argv)

    baseline_path = Path(args.baseline)
    current_path = Path(args.current)

    if not baseline_path.exists():
        print(f"baseline benchmark file not found: {baseline_path}", file=sys.stderr)
        return 1
    if not current_path.exists():
        print(f"current benchmark file not found: {current_path}", file=sys.stderr)
        return 1

    baseline = rows_by_plugin(baseline_path.read_text(encoding="utf-8"))
    current = rows_by_plugin(current_path.read_text(encoding="utf-8"))
    rows = compare_rows(baseline, current)
    markdown = format_markdown(
        rows,
        baseline_run_id=args.baseline_run_id,
        baseline_run_url=args.baseline_run_url,
    )

    if args.output:
        Path(args.output).write_text(markdown, encoding="utf-8")
    else:
        print(markdown, end="")

    if args.append_summary and "GITHUB_STEP_SUMMARY" in os.environ:
        append_to_step_summary(markdown)

    threshold: float | None = None
    regressions: list[dict[str, str]] = []

    if args.fail_on_regression_pct is not None:
        threshold = args.fail_on_regression_pct
        if threshold < 0:
            print("--fail-on-regression-pct must be >= 0", file=sys.stderr)
            return 1
        regressions = regressions_above_threshold(rows, threshold)
        if regressions:
            details = ", ".join(
                f"{row['plugin']} ({row['avg_delta_pct']})" for row in regressions
            )
            print(
                f"benchmark regressions above {threshold:.2f}%: {details}",
                file=sys.stderr,
            )
            if args.print_status_line:
                print(build_status_line(rows, threshold, regressions), file=sys.stderr)
            return 2

    if args.print_status_line:
        print(build_status_line(rows, threshold, regressions), file=sys.stderr)
    if args.write_github_output and "GITHUB_OUTPUT" in os.environ:
        write_github_output(status_values(rows, threshold, regressions))

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
