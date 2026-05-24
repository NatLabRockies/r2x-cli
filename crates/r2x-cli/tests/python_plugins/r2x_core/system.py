"""Minimal System model for integration tests."""

from __future__ import annotations

import json
from typing import Any


class System:
    """Stub System class with from_json API expected by runtime bridge."""

    def __init__(self, data: dict[str, Any]) -> None:
        self.data = data

    @classmethod
    def from_json(cls, payload: bytes | str) -> "System":
        if isinstance(payload, bytes):
            payload = payload.decode("utf-8")
        return cls(json.loads(payload))
