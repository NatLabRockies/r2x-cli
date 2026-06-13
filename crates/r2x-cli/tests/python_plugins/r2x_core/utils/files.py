"""Stub file helpers"""

from __future__ import annotations

from pathlib import Path


def ensure_directory(path: str | Path) -> Path:
    """No-op helper that matches the production API."""
    return Path(path)


def get_r2x_cache_path() -> Path:
    """Return a stub cache path (overridden at runtime by the Rust bridge)."""
    return Path.home() / ".config" / "r2x" / "cache"
