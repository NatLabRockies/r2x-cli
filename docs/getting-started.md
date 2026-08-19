# Getting Started

## Installation

Install the most recent `r2x-cli` release for your platform:

**macOS / Linux**

```bash
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/NatLabRockies/r2x-cli/releases/latest/download/r2x-installer.sh | sh
```

**Windows**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/NatLabRockies/r2x-cli/releases/latest/download/r2x-installer.ps1 | iex"
```

## Verify the Installation

```bash
r2x --version
```

## Update

If `r2x-cli` is already installed:

```bash
r2x self update
```

## Cache Management

Clean the R2X cache:

```bash
r2x cache clean
```

## Python & Virtual Environment

### Manage the Python version

```bash
r2x python show                  # show configured Python version and venv info
r2x python path                  # show Python executable path in the venv
r2x python install               # install the default Python version
r2x python install <version>     # install a specific version (e.g., 3.13)
```

### Manage the virtual environment

```bash
r2x venv create           # create or recreate the virtual environment
r2x venv create --yes     # skip confirmation prompt
r2x venv path             # show the venv path
r2x venv path <new-path>  # set a new venv path
```
