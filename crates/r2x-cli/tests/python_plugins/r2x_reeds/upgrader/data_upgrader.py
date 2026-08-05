"""Stub ReEDS upgrader used in integration tests."""

from __future__ import annotations

from pathlib import Path
from typing import Any


class ReEDSUpgrader:
    """Minimal upgrader that echoes a JSON payload."""

    steps: list[Any] = []

    def __init__(self, folder_path: Path | str, steps: list[Any] | None = None, **_: Any) -> None:
        self.folder_path = Path(folder_path)
        if steps is not None:
            self.steps = list(steps)

    def run(self) -> str:
        print("upgrader diagnostic")
        return f'{{"upgraded": "reeds", "folder": "{self.folder_path}"}}'
