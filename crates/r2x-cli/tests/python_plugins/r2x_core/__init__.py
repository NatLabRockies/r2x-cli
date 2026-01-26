"""Stub r2x_core package for integration tests."""

from typing import Any


class PluginContext:
    """Minimal PluginContext stub for integration tests."""

    def __init__(
        self,
        config: Any,
        *,
        store: Any | None = None,
        system: Any | None = None,
    ) -> None:
        self.config = config
        self.store = store
        self.system = system
