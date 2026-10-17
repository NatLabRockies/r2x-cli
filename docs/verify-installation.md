---
title: Verify the CLI and first use
---

After [Installation](installation.md) and [Python environment setup](environment.md):

```bash
r2x --version
r2x --help
r2x python show
r2x list
```

Expect the launcher version, available commands, runtime paths, and plugin discovery output.
A fresh environment may have no translator plugins; that is expected before installing a package.
If the launcher cannot find its runtime payload or Python, consult [Troubleshooting](troubleshooting.md).

Continue with [Run and inspect your first two-step pipeline](getting-started.md).
It installs a small real plugin package, doubles values 10 and 20, saves an artifact, and checks the reloaded total of 60.
For an existing translator package, follow [Manage plugins](plugin-management.md) instead.

## Understand installed plugin roles

The CLI discovers registrations from installed Python distributions; it does not ship a catalog of model translators.
Inspect a particular package with `r2x list PACKAGE`, then ask for each actual reference's help with `r2x run REFERENCE --help`.
These are role groups recognized by the [runtime mapper](https://github.com/NatLabRockies/r2x-cli/blob/main/crates/r2x-manifest/src/runtime.rs):

| Entry-point group | Runtime role | First check |
| --- | --- | --- |
| `r2x.parsers` | Parser | Inspect its input/configuration requirements before creating a system. |
| `r2x.exporters` | Exporter | Inspect the accepted system and output format. |
| `r2x.modifiers` | Modifier | Inspect the settings used to change an existing system. |
| `r2x.upgraders` | Upgrader | Check supported data versions before changing an input. |
| `r2x.transforms` or `r2x.translations` | Translation | Check the input and resulting system contracts. |

The shipped tutorial registers only `create` in `r2x.parsers` and `scale` in `r2x.transforms`.
There is no built-in `sysmod` command: modifiers are installed plugin references, run through `r2x run`.
A model package may provide additional registrations; its own Getting Started page describes them.
See [r2x-core](https://natlabrockies.github.io/r2x-core/) for lifecycle/configuration contracts and [r2x-reeds](https://natlabrockies.github.io/r2x-reeds/) for its package-specific inputs.
