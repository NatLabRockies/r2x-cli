---
title: Python environment setup
---

After [Installation](installation.md), install [uv](https://docs.astral.sh/uv/getting-started/installation/).
The CLI uses uv to manage its own Python environment and installed plugin packages.
Check `r2x python show` first if this installation may already have a working environment.
The following install command is for initial setup: it deletes and recreates an existing managed environment, including installed plugins, even if the Python version is unchanged.
For an existing installation, use the diagnosis and package-recording steps in [Troubleshooting](troubleshooting.md) before considering a rebuild.

```bash
r2x python install
r2x python show
```

`install` provisions the configured runtime; `show` reports its configuration and paths.
Use a Python major.minor ABI supported by your binary. Published binaries currently target Python 3.12; installing an arbitrary newer interpreter does not upgrade their ABI.
The exact flags and defaults are in the [CLI reference](cli-reference.md).

A project's uv environment and the CLI environment are separate.
Use `r2x install` for plugins that the CLI should discover; `uv add` in an unrelated project does not install plugins here.
See [plugin management](plugin-management.md) for package installation and discovery.
Git is only needed for repository examples or Git-based packages.
