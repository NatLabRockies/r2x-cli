#!/usr/bin/env python3
"""
Utility to locate the prefix path of a uv-managed Python installation.

Usage:
    python3 scripts/detect_uv_python.py [--version 3.12]

Prints the install prefix (parent of the `bin` directory) to stdout.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, Iterable, Optional

DEFAULT_VERSION = "3.12"


def load_uv_python_list(version: str) -> Iterable[Dict[str, Any]]:
    """Invoke `uv python list` and return parsed JSON entries."""
    result = subprocess.run(
        [
            "uv",
            "python",
            "list",
            "--only-installed",
            "--output-format",
            "json",
            version,
        ],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    data = json.loads(result.stdout)
    if not isinstance(data, list):
        raise RuntimeError("uv python list returned unexpected JSON payload")
    return data


def choose_uv_prefix(entries: Iterable[Dict[str, Any]]) -> Optional[Path]:
    """
    Pick the best python prefix directory from uv entries.

    Preference order:
      1. Paths under ~/.local/share/uv/python
      2. Paths containing AppData\\Local\\uv\\python (Windows)
      3. First available path entry
    """
    preferred: Optional[Path] = None
    fallback: Optional[Path] = None

    for entry in entries:
        raw_path = entry.get("path")
        if not raw_path:
            continue
        path = Path(raw_path)
        normalized = str(path).replace("\\", "/")

        if ".local/share/uv/python" in normalized or "AppData/Local/uv/python" in normalized:
            parent = path.parent  # bin directory
            return parent.parent

        if fallback is None:
            fallback = path.parent.parent

    return preferred or fallback


def main(argv: Optional[Iterable[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--version",
        default=os.environ.get("PY_VERSION", DEFAULT_VERSION),
        help="Python version to locate (default: %(default)s)",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    entries = load_uv_python_list(args.version)
    prefix = choose_uv_prefix(entries)
    if not prefix:
        print("error: unable to determine uv-managed python path", file=sys.stderr)
        return 1

    print(prefix)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
