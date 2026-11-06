"""Minimal DataStore implementation for integration tests."""

from __future__ import annotations

from pathlib import Path
from typing import Any


class DataStore:
    """Stub that only tracks the provided path."""

    def __init__(self, path: Path | str, **_: Any) -> None:
        self.path = Path(path)

    @classmethod
    def from_plugin_config(
        cls, plugin_config: Any, *, path: Path | str, **_: Any
    ) -> DataStore:
        store = cls(path)
        store.configure_upgrades(plugin_config=plugin_config)
        mapping_path = plugin_config.fmap_path
        if mapping_path.exists():
            store.load_file_mapping(mapping_path)
        return store

    def configure_upgrades(self, *, plugin_config: Any) -> None:
        self.plugin_config = plugin_config

    def load_file_mapping(self, mapping_path: Path) -> None:
        self.mapping_path = mapping_path
