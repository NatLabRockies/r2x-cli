#!/usr/bin/env python3
"""Format benchmark lines emitted by `r2x run plugin --benchmark` as Markdown."""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path


SUMMARY_PATTERN = re.compile(
    r"^Benchmark (?P<plugin>[^:]+): runs=(?P<runs>\d+) total=(?P<total>\S+) avg=(?P<avg>\S+)$"
)
BREAKDOWN_PATTERN = re.compile(
    r"^Benchmark (?P<plugin>[^:]+) breakdown: python=(?P<python>\S+) serialization=(?P<serialization>\S+) \(samples=(?P<samples>\d+)\)$"
)


def parse_benchmark_lines(content: str) -> list[dict[str, str]]:
    rows: dict[str, dict[str, str]] = {}
    for raw_line in content.splitlines():
        line = raw_line.strip()
        if not line.startswith("Benchmark "):
            continue

        summary = SUMMARY_PATTERN.match(line)
        if summary:
            plugin = summary.group("plugin")
            row = rows.setdefault(plugin, {"plugin": plugin})
            row.update(summary.groupdict())
            continue

        breakdown = BREAKDOWN_PATTERN.match(line)
        if breakdown:
            plugin = breakdown.group("plugin")
            row = rows.setdefault(plugin, {"plugin": plugin})
            row.update(breakdown.groupdict())

    return list(rows.values())


def format_markdown_table(rows: list[dict[str, str]]) -> str:
    if not rows:
        return "### Plugin Benchmark Summary\n\nNo benchmark lines were found.\n"

    header = [
        "### Plugin Benchmark Summary",
        "",
        "| Plugin | Runs | Total | Avg | Python | Serialization | Samples |",
        "| --- | ---: | ---: | ---: | ---: | ---: | ---: |",
    ]
    body = []
    for row in rows:
        body.append(
            "| {plugin} | {runs} | {total} | {avg} | {python} | {serialization} | {samples} |".format(
                plugin=row.get("plugin", "unknown"),
                runs=row.get("runs", "n/a"),
                total=row.get("total", "n/a"),
                avg=row.get("avg", "n/a"),
                python=row.get("python", "n/a"),
                serialization=row.get("serialization", "n/a"),
                samples=row.get("samples", "n/a"),
            )
        )
    return "\n".join(header + body) + "\n"


def append_to_step_summary(markdown: str) -> None:
    summary_path = Path(os.environ["GITHUB_STEP_SUMMARY"])
    with summary_path.open("a", encoding="utf-8") as handle:
        handle.write("\n")
        handle.write(markdown)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, help="Path to benchmark text output")
    parser.add_argument("--output", help="Optional path to write formatted markdown")
    parser.add_argument(
        "--append-summary",
        action="store_true",
        help="Append markdown output to GITHUB_STEP_SUMMARY when available",
    )
    args = parser.parse_args(argv)

    input_path = Path(args.input)
    if not input_path.exists():
        print(f"benchmark input file not found: {input_path}", file=sys.stderr)
        return 1

    rows = parse_benchmark_lines(input_path.read_text(encoding="utf-8"))
    markdown = format_markdown_table(rows)

    if args.output:
        Path(args.output).write_text(markdown, encoding="utf-8")
    else:
        print(markdown, end="")

    if args.append_summary and "GITHUB_STEP_SUMMARY" in os.environ:
        append_to_step_summary(markdown)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
