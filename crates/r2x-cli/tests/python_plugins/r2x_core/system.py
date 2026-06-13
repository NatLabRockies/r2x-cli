"""Minimal System model for integration tests."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


class System:
    """Stub System class with JSON APIs expected by runtime bridges."""

    DB_FILENAME = "time_series_metadata.db"

    def __init__(self, data: dict[str, Any]) -> None:
        self.data = data

    @classmethod
    def from_dict(cls, data: dict[str, Any], time_series_parent_dir: str | Path) -> "System":
        time_series = data.get("time_series")
        if isinstance(time_series, dict) and time_series.get("directory"):
            sidecar_dir = Path(time_series["directory"])
            if not sidecar_dir.is_absolute():
                sidecar_dir = Path(time_series_parent_dir) / sidecar_dir
            if not sidecar_dir.exists() or not (sidecar_dir / cls.DB_FILENAME).exists():
                raise OSError("unable to open database file")
        return cls(data)

    @classmethod
    def from_json(cls, payload: bytes | str) -> "System":
        if isinstance(payload, bytes):
            payload = payload.decode("utf-8")
        return cls(json.loads(payload))
