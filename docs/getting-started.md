---
title: r2x-cli Getting Started
---

# r2x-cli Getting Started

This guide installs `r2x-cli`, initializes a workspace, and gets a plugin ready for
a pipeline. For exact command and flag behavior, see the [CLI
reference](cli-reference.md).

## Install

### macOS and Linux

```bash
curl -fsSL https://github.com/NatLabRockies/r2x-cli/releases/latest/download/r2x-installer.sh | sh
```

The installer places the `r2x` command and its adjacent runtime payload in
`~/.local/bin` by default. Restart your shell, or source the environment file
shown by the installer, if `r2x` is not immediately available.

### Windows

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/NatLabRockies/r2x-cli/releases/latest/download/r2x-installer.ps1 | iex"
```

### Verify

```bash
r2x --version
```

`r2x --version` only checks the launcher. Commands that install or run Python
plugins also need `uv` and a managed Python environment. On first use,
`r2x-cli` will offer to install `uv` with its official installer when `uv` is
not found.
The default managed Python version is selected when the binary is built,
usually Python 3.12 for release builds.

## Initialize a workspace

Create a pipeline template in a new or existing working directory:

```bash
mkdir my-r2x-workspace
cd my-r2x-workspace
r2x init
```

This creates `pipeline.yaml`. Replace its example plugin names and settings
with the plugins and input files for your workflow.

## Install and inspect plugins

Install a plugin from PyPI:

```bash
r2x install r2x-reeds
r2x list
```

Install a plugin from GitHub with a branch, tag, or commit:

```bash
r2x install --branch main gh:NatLabRockies/r2x-reeds
```

See [Plugin management](plugin-management.md) for editable installs, Git
URLs, subdirectories, cache behavior, and upgrades.

## Validate and run a pipeline

List the pipelines in a configuration file:

```bash
r2x run pipeline.yaml --list
```

Replace `my-pipeline` below with a name returned by `--list`.
Inspect its resolved configuration, then preview the execution plan without running plugins:

```bash
r2x run pipeline.yaml my-pipeline --print
r2x run pipeline.yaml my-pipeline --dry-run
```

A successful preview verifies the plan, not the availability or correctness of model input data.
Supply the input files and settings required by the selected plugins before running it:

```bash
r2x run pipeline.yaml my-pipeline
```

## Update

For a standalone installer installation, update to the latest release with:

```bash
r2x self update
```

The `upgrade` alias is also available:

```bash
r2x self upgrade
```

If `r2x-cli` was installed with Cargo, Homebrew, or another package manager, use
that package manager's update command instead.

## Manage the runtime

Show the configured Python version and managed environment:

```bash
r2x python show
```

Install the configured Python version matching your binary:

```bash
r2x python install
```

An explicit version such as `r2x python install 3.13` requires a CLI binary built for the same Python major.minor ABI.
See [source builds](development.md#source-build-with-another-python-version) before changing it.

Show the Python executable used by the managed environment:

```bash
r2x python path
```

Create or recreate the managed virtual environment after changing its Python
ABI:

```bash
r2x venv create --yes
```

Set or inspect the virtual environment path:

```bash
r2x venv path
r2x venv path /path/to/r2x-venv
```
