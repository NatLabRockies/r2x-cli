"""Stub Sienna upgrader for integration tests."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any


class SiennaUpgrader:
    """Upgrader that mutates the JSON at the provided path."""

    def __init__(self, path: Path | str, **_: Any) -> None:
        self.path = Path(path)

    def run(self) -> str:
        data = json.loads(self.path.read_text(encoding="utf-8"))
        data["upgraded"] = "sienna"
        data["path"] = str(self.path)
        output = json.dumps(data)
        self.path.write_text(output, encoding="utf-8")
        return output
