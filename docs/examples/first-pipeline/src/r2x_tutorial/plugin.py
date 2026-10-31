"""A class parser and exposed function transform demonstrating system handoff."""

from infrasys import Component
from pydantic import Field
from rust_ok import Ok, Result

from r2x_core import Plugin, PluginConfig, System, expose_plugin


class Item(Component):
    value: float


class CreateConfig(PluginConfig):
    name: str = "demo"


class CreateDemo(Plugin[CreateConfig]):
    def on_build(self) -> Result[System, str]:
        system = System(name=self.config.name)
        system.add_component(Item(name="one", value=10))
        system.add_component(Item(name="two", value=20))
        return Ok(system)


class ScaleConfig(PluginConfig):
    factor: float = Field(default=2, gt=0)


@expose_plugin
def scale_values(system: System, config: ScaleConfig) -> Result[System, str]:
    for item in system.get_components(Item):
        item.value *= config.factor
    return Ok(system)
