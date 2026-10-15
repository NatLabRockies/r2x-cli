<img src="./assets/r2x-logo-full-square.svg" alt="R2X framework logo" align="left" width="220px" height="220px" hspace="10"/>
<img align="left" alt="" width="0" height="220px" hspace="10"/>

#### r2x-cli
<p><small>Plugin manager and pipeline runner for model interoperability.</small></p>

[![CI](https://github.com/NatLabRockies/r2x-cli/actions/workflows/build.yml/badge.svg)](https://github.com/NatLabRockies/r2x-cli/actions/workflows/build.yml) [![Latest release](https://img.shields.io/github/v/release/NatLabRockies/r2x-cli?display_name=tag&label=latest%20release&color=0079c2&logo=github)](https://github.com/NatLabRockies/r2x-cli/releases/latest) [![Last commit](https://img.shields.io/github/last-commit/NatLabRockies/r2x-cli?style=flat-square)](https://github.com/NatLabRockies/r2x-cli/commits/main) [![Docs](https://img.shields.io/badge/docs-guides-0079c2?style=flat-square)](docs/README.md)
<br/>
[![Rust 1.72+](https://img.shields.io/badge/Rust-1.72%2B-dea584?logo=rust&logoColor=white)](docs/development.md) [![Python 3.11+](https://img.shields.io/badge/Python-3.11%2B-3776ab?logo=python&logoColor=white)](docs/development.md) [![Managed with uv](https://img.shields.io/badge/managed%20with-uv-6f42c1)](https://docs.astral.sh/uv/) [![BSD 3-Clause](https://img.shields.io/badge/license-BSD--3--Clause-blue.svg)](./LICENSE.txt)

<br clear="left">

`r2x-cli` is a Rust CLI that discovers Python plugins, composes them into
repeatable pipelines, and manages the Python runtime with `uv`.
It keeps model data and sidecar artifacts together across process boundaries.

## Quick Start

### Install

macOS and Linux:

```bash
curl -fsSL https://github.com/NatLabRockies/r2x-cli/releases/latest/download/r2x-installer.sh | sh
```

Windows PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/NatLabRockies/r2x-cli/releases/latest/download/r2x-installer.ps1 | iex"
```

Verify the installation:

```bash
r2x --version
```

## Documentation

Use the guide that matches your task:

- [Getting started](docs/getting-started.md): install, initialize, and run a pipeline.
- [CLI reference](docs/cli-reference.md): commands, flags, runtime behavior, and help.
- [Plugin management](docs/plugin-management.md): install, inspect, upgrade, and remove plugins.
- [Architecture](docs/architecture.md): Rust crates, runtime boundaries, discovery, and artifacts.
- [Development](docs/development.md): source builds, checks, Python ABIs, and troubleshooting.

- [Troubleshooting](docs/troubleshooting.md): diagnose runtime, discovery, and pipeline failures.
- [Changelog and versions](docs/changelog.md): canonical releases and tagged source docs.

See the [documentation index](docs/README.md) for the complete guide map.

## Development

Install Rust, `uv`, Python 3.11 or newer, and `just`.
Run the aggregate check with:

```bash
just all
```

See [Development](docs/development.md) for targeted formatting, lint, build, and test commands.

## License

BSD-3-Clause.
See [LICENSE.txt](./LICENSE.txt) for the full license text.
