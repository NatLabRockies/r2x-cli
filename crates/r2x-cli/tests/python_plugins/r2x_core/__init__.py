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


def run_upgrade_step(
    data: Any, *, step: Any, upgrader_context: Any | None = None
) -> Any:
    """Expose the current r2x-core upgrader call contract."""
    from .upgrader_utils import run_upgrade_step as run_step

    return run_step(data, step=step, upgrader_context=upgrader_context)
