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

    def to_json(self, path: str | Path | None = None) -> bytes | None:
        payload = json.dumps(self.data, ensure_ascii=False)
        if path is None:
            return payload.encode("utf-8")
        output = Path(path)
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(payload, encoding="utf-8")
        time_series = self.data.get("time_series")
        if isinstance(time_series, dict) and time_series.get("directory"):
            sidecar_dir = Path(time_series["directory"])
            if not sidecar_dir.is_absolute():
                sidecar_dir = output.parent / sidecar_dir
            sidecar_dir.mkdir(parents=True, exist_ok=True)
            (sidecar_dir / self.DB_FILENAME).write_text("sidecar", encoding="utf-8")
        return None

    @classmethod
    def from_json(cls, source: bytes | str | Path) -> "System":
        if isinstance(source, bytes):
            return cls(json.loads(source.decode("utf-8")))
        if isinstance(source, Path):
            data = json.loads(source.read_text(encoding="utf-8"))
            return cls.from_dict(data, source.parent)
        if source.lstrip().startswith(("{", "[", '"')):
            return cls(json.loads(source))
        path = Path(source)
        return cls.from_dict(json.loads(path.read_text(encoding="utf-8")), path.parent)
