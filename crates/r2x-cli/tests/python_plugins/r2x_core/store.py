"""Minimal DataStore implementation for integration tests."""

from __future__ import annotations

from pathlib import Path
from typing import Any


class DataStore:
    """Stub that only tracks the provided path."""

    def __init__(self, path: Path | str, **_: Any) -> None:
        self.path = Path(path)

    def configure_upgrades(self, *, plugin_config: Any) -> None:
        self.plugin_config = plugin_config

    def load_file_mapping(self, mapping_path: Path) -> None:
        self.mapping_path = mapping_path
