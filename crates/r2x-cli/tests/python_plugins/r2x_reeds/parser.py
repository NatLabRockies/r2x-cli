"""Stub ReEDS parser used in integration tests."""

from __future__ import annotations

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
