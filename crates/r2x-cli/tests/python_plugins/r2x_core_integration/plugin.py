"""Plugins used to exercise the CLI with an installed r2x-core."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from r2x_core import (
    DataStore,
    PluginConfig,
    PluginContext,
    System,
    UpgradeStep,
    UpgradeType,
)


class CoreConfig(PluginConfig):
    """Config model for the configured-store integration case."""


class CoreSystemPlugin:
    """Return an actual r2x-core System after reading its configured store."""

    def __init__(self, context: PluginContext[CoreConfig]) -> None:
        self.context = context

    @classmethod
    def from_context(cls, context: PluginContext[CoreConfig]) -> CoreSystemPlugin:
        """Construct the plugin from the CLI-provided context."""
        return cls(context)

    def build_system(self) -> System:
        """Require a loaded mapping and return a real core System."""
        store = self.context.store
        if not isinstance(store, DataStore):
            raise TypeError("CLI did not provide an r2x-core DataStore")
        if store.list_data() != ["input"]:
            raise ValueError(f"unexpected configured store entries: {store.list_data()}")
        if store["input"].relative_fpath != Path("input.csv"):
            raise ValueError("CLI loaded an unexpected input mapping")
        return System(name="cli-r2x-core-integration")


def mark_upgraded(data: dict[str, Any], upgrader_context: Any | None = None) -> dict[str, Any]:
    """Mark the system payload to verify public upgrade-step dispatch."""
    data["status"] = "upgraded-by-r2x-core"
    return data


_SYSTEM_UPGRADE = UpgradeStep(
    name="mark-system-upgraded",
    func=mark_upgraded,
    target_version="1",
    upgrade_type=UpgradeType.SYSTEM,
)


class CoreUpgrader:
    """Expose a registered step without a run method to exercise CLI fallback."""

    steps = [_SYSTEM_UPGRADE]

    def __init__(self, path: Path | str) -> None:
        self.path = Path(path)
