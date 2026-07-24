"""Stub ReEDS parser used in integration tests."""

from __future__ import annotations

import json
from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from r2x_core import PluginContext


class ReEDSConfig:
    """Minimal config placeholder matching runtime expectations."""

    def __init__(
        self,
        weather_year: int | None = None,
        solve_year: int | None = None,
        **kwargs: Any,
    ) -> None:
        self.weather_year = weather_year
        self.solve_year = solve_year
        self.extra = kwargs


class ReEDSParser:
    """Parser that returns canned JSON for tests."""

    def __init__(
        self, config: ReEDSConfig | None = None, data_store: Any | None = None, **_: Any
    ) -> None:
        self.config = config
        self.data_store = data_store

    @classmethod
    def from_context(cls, ctx: PluginContext) -> ReEDSParser:
        """Create parser instance from a PluginContext."""
        return cls(config=ctx.config, data_store=ctx.store)

    def build_system(self) -> str:
        return '{"system": "reeds", "status": "ok"}'


def no_stdin_function() -> dict[str, str]:
    """Function plugin intentionally omits stdin/system params."""
    return {"system": "reeds", "status": "function-no-stdin"}


def with_system_function(system) -> dict[str, str]:
    """Function plugin that requires system/stdin deserialization."""
    data = getattr(system, "data", {})
    if isinstance(data, str):
        try:
            data = json.loads(data)
        except json.JSONDecodeError:
            data = {}
    source = data.get("system", "unknown")
    return {"system": source, "status": "function-with-system"}


def system_with_sidecar():
    """Return a path-serialized System with a relative sidecar bundle."""
    from r2x_core.system import System

    return System(
        {
            "system": "reeds",
            "status": "with-sidecar",
            "time_series": {"directory": "system_time_series"},
        }
    )
