# r2x-cli

A framework plugin manager for the r2x power systems modeling ecosystem. The r2x-cli simplifies discovery, installation, and management of r2x framework plugins, providing a unified interface for running data processing pipelines.

## Installation

Download the latest pre-built binary for your platform from the [releases page](https://github.com/NatLabRockies/r2x-cli/releases/latest).

## Building from Source

### Prerequisites

Building r2x-cli requires the Rust toolchain, the uv package manager, and Python 3.11, 3.12, or 3.13.

Install the Rust toolchain via rustup:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Install the uv package manager:

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

Install Python via uv:

```bash
uv python install 3.12
```

Restart your shell after installation to ensure the tools are available in your PATH.

### Build and Install

Clone the repository:

```bash
git clone https://github.com/NatLabRockies/r2x-cli && cd r2x-cli
```

Build and install using cargo. The `PYO3_PYTHON` environment variable tells PyO3 which Python interpreter to use:

```bash
PYO3_PYTHON=$(uv python find 3.12) cargo install --path crates/r2x-cli --force --locked
```

This installs the `r2x` binary to `~/.cargo/bin/`, which should already be in your PATH if you installed Rust via rustup.

Verify the installation:

```bash
r2x --version
r2x --help
```

### Alternative: Manual Build

For more control over the build process, you can build manually and place the binary in a custom location:

```bash
PYO3_PYTHON=$(uv python find 3.12) cargo build --release
```

The binary will be at `target/release/r2x`. Copy it to your preferred location and ensure that location is in your PATH.

### Troubleshooting Build Issues

If the build fails with a Python-related error, verify that `uv python find 3.12` returns a valid path. You may need to run `uv python install 3.12` first.

If the r2x command is not found after installation, verify `~/.cargo/bin` is in your PATH with `echo $PATH`.

On HPC systems or machines with older glibc versions, building from source is often required because pre-built binaries may be incompatible.

## Getting Started

Initialize a new pipeline configuration file:

```bash
r2x init
```

This creates a `pipeline.yaml` file containing example variables, pipeline definitions, and plugin configuration templates. Specify a custom filename with `r2x init my-pipeline.yaml`.

## Configuration

Display current configuration:

```bash
r2x config show
```

Update configuration values:

```bash
r2x config set python-version 3.13
r2x config set cache-path /path/to/cache
```

Reset configuration to defaults:

```bash
r2x config reset -y
```

### Python and Virtual Environment

Install a specific Python version:

```bash
r2x config python install 3.13
```

Get the Python executable path:

```bash
r2x config python path
```

Create or recreate the virtual environment:

```bash
r2x config venv create -y
```

Install packages into the managed virtual environment:

```bash
uv pip install <package> --python $(r2x config python path)
```

### Cache

Clean the cache directory:

```bash
r2x config cache clean
```

## Plugin Management

List all installed plugins:

```bash
r2x list
```

Filter by package or module name:

```bash
r2x list r2x-reeds
r2x list r2x-reeds break_gens
```

Install a plugin from PyPI:

```bash
r2x install r2x-reeds
```

Install from a git repository:

```bash
r2x install NREL/r2x-reeds
r2x install --branch develop NREL/r2x-reeds
r2x install --tag v1.0.0 NREL/r2x-reeds
```

Install in editable mode for development:

```bash
r2x install -e /path/to/local/plugin
```

Remove a plugin:

```bash
r2x remove r2x-reeds
```

Re-run plugin discovery:

```bash
r2x sync
```

Clean the plugin manifest:

```bash
r2x clean -y
```

## Running Plugins

Run a plugin with arguments:

```bash
r2x run plugin r2x_reeds.parser store-path=/path/to/data solve_year=2030
```

Show plugin help:

```bash
r2x run plugin r2x_reeds.parser --show-help
```

List available plugins:

```bash
r2x run plugin
```

## Pipeline Management

List all pipelines in a configuration file:

```bash
r2x run pipeline.yaml --list
```

Preview pipeline execution without running:

```bash
r2x run pipeline.yaml my-pipeline --dry-run
```

Execute a pipeline:

```bash
r2x run pipeline.yaml my-pipeline
```

Execute and save output:

```bash
r2x run pipeline.yaml my-pipeline -o output.json
```

### Pipeline Configuration Format

Pipeline configuration files use YAML with three sections: `variables` for substitution values, `pipelines` for named plugin sequences, and `config` for plugin-specific settings.

```yaml
variables:
  output_dir: "output"
  reeds_run: /path/to/reeds/run
  solve_year: 2032

pipelines:
  reeds-test:
    - r2x-reeds.parser
    - r2x-reeds.break-gens

  reeds-to-plexos:
    - r2x-reeds.upgrader
    - r2x-reeds.parser
    - r2x-reeds-to-plexos.translation
    - r2x-plexos.exporter

config:
  r2x-reeds.upgrader:
    path: ${reeds_run}

  r2x-reeds.parser:
    weather_year: 2012
    solve_year: ${solve_year}
    path: ${reeds_run}

  r2x-reeds.break-gens:
    drop_capacity_threshold: 5

  r2x-plexos.exporter:
    output: ${output_dir}

output_folder: ${output_dir}
```

Run a pipeline with:

```bash
r2x run pipeline.yaml reeds-test
```

## Interactive System Shell

Load a system from JSON and open an interactive IPython session:

```bash
r2x read system.json
```

Load from stdin:

```bash
cat system.json | r2x read
```

Execute a script against the loaded system:

```bash
r2x read system.json --exec script.py
```

Execute a script then drop into interactive mode:

```bash
r2x read system.json --exec script.py -i
```

The interactive session provides the loaded system as `sys`, access to plugins via `plugins`, and lazy-loaded `pd`, `np`, and `plt` for pandas, numpy, and matplotlib. Type `%r2x_help` in the session to see available commands.

## Verbosity Control

Suppress informational logs:

```bash
r2x -q run plugin my-plugin
```

Suppress logs and plugin stdout:

```bash
r2x -qq run plugin my-plugin
```

Enable debug logging:

```bash
r2x -v run plugin my-plugin
```

Enable trace logging:

```bash
r2x -vv run plugin my-plugin
```

Show Python logs on console:

```bash
r2x --log-python run plugin my-plugin
```

## License

BSD-3-Clause License. See [LICENSE.txt](LICENSE.txt) for details.
