"""Run with r2x read --exec inspect_system.py output/system.json."""

from r2x_tutorial.plugin import Item

from r2x_core import System

system: System = globals()["sys"]
items = sorted(system.get_components(Item), key=lambda item: item.name)
assert [item.value for item in items] == [20, 40]
print(f"{len(items)} items; total={sum(item.value for item in items):.1f}")
