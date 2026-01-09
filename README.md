# r2x-cli

A comprehensive framework plugin manager for the r2x power systems modeling ecosystem. Simplifies discovery, installation, and management of r2x framework plugins.

## Features

- Easy plugin management
- Built-in package resolution
- Python integration support

## Building from Source

### Prerequisites

Before building r2x-cli, you need:

1. **Rust toolchain** - Install via rustup:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```
   Follow the prompts and choose the default installation method.

2. **UV package manager** - Install via:
   ```bash
   curl -LsSf https://astral.sh/uv/install.sh | sh
   ```

3. **Python 3.11, 3.12, or 3.13** - Install via uv:
   ```bash
   uv python install 3.12
   ```

After installation, restart your shell to ensure the tools are in your PATH.

### Build Steps

#### Step 1: Clone and Build

Clone the repository:
```bash
# For users (HTTPS)
git clone https://github.com/NREL/r2x-cli && cd r2x-cli

# For developers (SSH)
git clone git@github.com:NREL/r2x-cli.git && cd r2x-cli
```

Set the `PYO3_PYTHON` environment variable to point to your Python installation. This is required for the PyO3 bindings to find the correct Python interpreter:

```bash
# Find your Python installation path
uv python list

# Linux example:
export PYO3_PYTHON=~/.local/share/uv/python/cpython-3.12.11-linux-x86_64-gnu/bin/python3.12

# macOS example:
export PYO3_PYTHON=~/.local/share/uv/python/cpython-3.12.11-macos-aarch64-none/bin/python3.12

# Windows example (PowerShell):
$env:PYO3_PYTHON = "$env:USERPROFILE\.local\share\uv\python\cpython-3.12.11-windows-x86_64-none\python.exe"
```

Build in release mode:
```bash
cargo build --release
```

The binary will be available at `target/release/r2x` (or `target/release/r2x.exe` on Windows).

#### Step 2: Install the Binary

Create the r2x binary directory and copy the built binary:

```bash
# Linux/macOS
mkdir -p ~/.r2x/bin
cp target/release/r2x ~/.r2x/bin/r2x

# Windows (PowerShell)
New-Item -ItemType Directory -Force -Path "$env:USERPROFILE\.r2x\bin"
Copy-Item target\release\r2x.exe "$env:USERPROFILE\.r2x\bin\r2x.exe"
```

#### Step 3: Link Python Dynamic Library

The r2x-cli binary needs to find the Python shared library at runtime. Create a symbolic link in the binary directory:

**Linux:**
```bash
# Link libpython3.12.so.1.0
ln -s ~/.local/share/uv/python/cpython-3.12.11-linux-x86_64-gnu/lib/libpython3.12.so.1.0 \
      ~/.r2x/bin/libpython3.12.so.1.0

# If the file doesn't exist, check the exact path with:
find ~/.local/share/uv/python -name "libpython*.so*"
```

**macOS:**
```bash
# Link libpython3.12.dylib
ln -s ~/.local/share/uv/python/cpython-3.12.11-macos-aarch64-none/lib/libpython3.12.dylib \
      ~/.r2x/bin/libpython3.12.dylib

# For Intel Macs, the path may be different:
find ~/.local/share/uv/python -name "libpython*.dylib"
```

**Windows:**
```powershell
# Copy python312.dll (Windows uses DLL instead of symlinks)
Copy-Item "$env:USERPROFILE\.local\share\uv\python\cpython-3.12.11-windows-x86_64-none\python312.dll" `
          "$env:USERPROFILE\.r2x\bin\python312.dll"
```

#### Step 4: Update PATH

Add the r2x binary directory to your PATH:

**Linux/macOS (bash):**
```bash
echo 'export PATH="$HOME/.r2x/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

**Linux/macOS (zsh):**
```bash
echo 'export PATH="$HOME/.r2x/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

**Windows (PowerShell):**
```powershell
# Add to user PATH permanently
[Environment]::SetEnvironmentVariable(
    "Path",
    [Environment]::GetEnvironmentVariable("Path", "User") + ";$env:USERPROFILE\.r2x\bin",
    "User"
)

# For current session only:
$env:PATH = "$env:USERPROFILE\.r2x\bin;$env:PATH"
```

#### Step 5: Verify Installation

Test that r2x is working:
```bash
r2x --version
r2x --help
```

You should see the version information and list of available commands.

### Troubleshooting Build Issues

**Problem: "libpython not found" error**
- Verify the symbolic link or DLL copy was created correctly
- Check that the Python library exists at the source path using `find` (Linux/macOS) or `Get-ChildItem` (Windows)
- Ensure the Python version in the library filename matches your PYO3_PYTHON version

**Problem: Build fails with "PYO3_PYTHON not set"**
- Make sure you exported PYO3_PYTHON before running `cargo build`
- Verify the path points to a valid Python executable: `$PYO3_PYTHON --version`

**Problem: r2x command not found**
- Verify ~/.r2x/bin is in your PATH: `echo $PATH`
- Try using the full path: `~/.r2x/bin/r2x --version`
- Make sure you reloaded your shell configuration

**HPC/Older Systems:**
On HPC systems or machines with older glibc versions, building from source is often required because pre-built binaries may be incompatible. Follow the steps above, paying special attention to setting the correct PYO3_PYTHON path for your system's Python installation.

## Python Package Management

This project uses [uv](https://github.com/astral-sh/uv) for handling Python packages. With uv, you don't need to worry about Python or plugin versioning—uv automatically handles dependency resolution and environment isolation for you.

### Getting Started

```bash
# Initialize a new pipeline file in the current directory
r2x init

# Initialize with custom filename
r2x init my-pipeline.yaml
```

The `r2x init` command creates a template pipeline file with:

- Example variables for substitution
- Multiple pipeline examples
- Plugin configuration templates
- Comments explaining all features

You can then edit the file to configure your own pipelines.

### Configuration Management

```bash
# Show current configuration
r2x config show

# Set configuration values
r2x config set python-version 3.13
r2x config set cache-path /path/to/cache

# View or set config file path
r2x config path                    # Show path
r2x config path /new/config/path   # Set path

# Reset configuration to defaults
r2x config reset -y
```

### Python & Virtual Environment Management

```bash
# Install or update Python version
r2x config python install 3.13

# Get the Python executable path (useful for scripting)
r2x config python path

# Show current Python configuration
r2x config python show

# Manage virtual environment
r2x config venv create              # Recreate venv (prompts for confirmation)
r2x config venv create -y/--yes     # Skip confirmation
r2x config venv path                # Show venv path

# Get or set custom venv path
r2x config venv path /path/to/custom/venv

# Automated venv recreation
R2X_VENV_YES=1 r2x config venv create  # Skip confirmation with environment variable
```

**Install packages in your managed venv:**

```bash
uv pip install <package> --python $(r2x config python path)
```

### Cache Management

```bash
# Clean the cache directory
r2x config cache clean

# View or set cache path
r2x config cache path                    # Show path
r2x config cache path /new/cache/path    # Set path
```

### Pipeline Management

```bash
# List available pipelines in a file
r2x run pipeline.yaml --list

# Execute a pipeline (explicit path and name)
r2x run pipeline.yaml my-pipeline

# Execute from default pipeline.yaml
r2x run my-pipeline

# Show pipeline structure
r2x run pipeline.yaml my-pipeline --print

# Show pipeline flow without executing (--dry-run)
r2x run pipeline.yaml my-pipeline --dry-run

# Save pipeline output to file
r2x run pipeline.yaml my-pipeline -o output.json
```

The `--dry-run` flag displays which plugins produce/consume stdout, helping you understand data flow between pipeline stages before execution.

### Plugin Management

```bash
# List all installed plugins
r2x list

# Filter by plugin package name
r2x list r2x-reeds

# Filter by module/function name
r2x list r2x-reeds break_gens

# Install a plugin from PyPI
r2x install r2x-reeds

# Install from git repository
r2x install NREL/r2x-reeds

# Install from git with custom host
r2x install --host github.com NREL/r2x-reeds

# Install from specific branch, tag, or commit
r2x install --branch develop NREL/r2x-reeds
r2x install --tag v1.0.0 NREL/r2x-reeds
r2x install --commit abc123 NREL/r2x-reeds

# Install in editable mode (for development)
r2x install -e /path/to/plugin

# Install with cache disabled (force rebuild)
r2x install --no-cache r2x-reeds

# Remove a plugin
r2x remove r2x-reeds

# Sync plugin manifest (refresh plugin discovery)
r2x sync

# Clean plugin manifest (remove all plugins)
r2x clean
r2x clean -y  # Skip confirmation
```

### Running Plugins Directly

```bash
# Run a plugin with arguments (key=value format)
r2x run plugin my-plugin key1=value1 key2=value2

# Show plugin help and parameters
r2x run plugin my-plugin --show-help

# Example: Run ReEDS parser
r2x run plugin r2x_reeds.parser \
  store-path=/path/to/reeds/run \
  solve_year=2030 \
  weather_year=2012 \
  case_name=my_scenario

# Pass additional arguments after plugin args
r2x run plugin my-plugin arg1=value1 -- --extra-flag
```

### System Integration

```bash
# Load and inspect a JSON system file
r2x read system.json

# Load from stdin and open IPython with system available
cat system.json | r2x read

# Pipe plugin output to read command
r2x run plugin r2x_reeds.parser store-path=/data | r2x read
```

### Verbosity Control

```bash
# Quiet mode (suppress info logs)
r2x -q run plugin my-plugin

# Very quiet mode (suppress info and plugin stdout)
r2x -q -q run plugin my-plugin

# Verbose mode (show debug logs)
r2x -v run plugin my-plugin

# Very verbose mode (show trace logs)
r2x -vv run plugin my-plugin

# Enable Python logging
r2x --log-python run plugin my-plugin
```

## License

BSD-3-Clause License. See [LICENSE.txt](LICENSE.txt) for details.
