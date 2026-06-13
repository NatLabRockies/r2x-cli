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


def validate_python_version(version: str) -> str:
    """Return a supported Python version string or raise ValueError."""
    version = version.strip()
    parts = version.split(".")
    if len(parts) not in (2, 3) or any(not part.isdigit() for part in parts):
        raise ValueError(
            f"expected a Python version like 3.12 or 3.12.1, got: {version}"
        )

    major = int(parts[0])
    minor = int(parts[1])
    if major != 3 or minor < 11:
        raise ValueError(
            f"Python {version} is not supported; r2x requires Python 3.11 or newer"
        )

    return version


def python_abi_version(version: str) -> str:
    """Normalize 3.x(.patch) to 3.x ABI form."""
    parts = version.strip().split(".")
    return f"{parts[0]}.{parts[1]}"


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
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"uv python list failed for {version}: {result.stderr.strip() or f'exit {result.returncode}'}"
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
        default=os.environ.get("R2X_PYTHON_VERSION")
        or os.environ.get("PY_VERSION")
        or DEFAULT_VERSION,
        help="Python version to locate (default: %(default)s)",
    )
    args = parser.parse_args(list(argv) if argv is not None else None)

    try:
        version = validate_python_version(args.version)
    except ValueError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    is_patch_request = version.count(".") == 2
    fallback_error: Optional[Exception] = None
    try:
        entries = load_uv_python_list(version)
        prefix = choose_uv_prefix(entries)
    except Exception as error:
        entries = []
        prefix = None
        fallback_error = error

    if (not prefix) and is_patch_request:
        requested_abi = python_abi_version(version)
        if requested_abi != version:
            try:
                entries = load_uv_python_list(requested_abi)
                prefix = choose_uv_prefix(entries)
            except Exception:
                prefix = None

    if not prefix and fallback_error is not None and not is_patch_request:
        print(f"error: {fallback_error}", file=sys.stderr)
        return 1

    if not prefix:
        if fallback_error is not None and is_patch_request:
            print(
                f"error: unable to determine uv-managed python path (requested {version}, fallback ABI {python_abi_version(version)})",
                file=sys.stderr,
            )
            return 1
        print("error: unable to determine uv-managed python path", file=sys.stderr)
        return 1

    print(prefix)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
