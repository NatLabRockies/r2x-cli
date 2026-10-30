---
title: r2x-cli Development
---

# r2x-cli Development

This page covers building `r2x-cli` from source and running the repository checks.
The release installer is the recommended path for users who do not need to
modify the CLI.

## Prerequisites

Install:

- Rust through [rustup](https://rustup.rs/)
- [uv](https://docs.astral.sh/uv/)
- Python 3.11 or newer through `uv`
- [just](https://just.systems/) for the repository task shortcuts

The build embeds the Python ABI selected by `R2X_PYTHON_VERSION`. Release
builds currently use Python 3.12. A source build can select another supported
major.minor version, but `PYO3_PYTHON` and `R2X_PYTHON_VERSION` must refer to the
same Python ABI.

## Build and install

Clone the repository and install a managed Python interpreter:

```bash
git clone https://github.com/NatLabRockies/r2x-cli
cd r2x-cli
uv python install 3.12
```

Build and install both the launcher and runtime payload:

```bash
PYO3_PYTHON="$(uv python find --no-config --no-project --managed-python 3.12)" \
R2X_PYTHON_VERSION=3.12 \
cargo install --path crates/r2x-cli --bins --force --locked
```

The binaries are installed together in `~/.cargo/bin/`. Keep the `r2x` command and
`r2x-runtime` in the same directory when moving the installation.

Verify the build:

```bash
r2x --version
```

## Repository checks

The `justfile` supplies the normal development commands:

```bash
just fmt       # format Rust sources
just clippy    # run Clippy
just build     # build the workspace
just test      # run workspace tests with the configured Python library path
just lint      # format and run Clippy
just all       # format, Clippy, and tests
```

The CI-equivalent formatting and lint checks are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Run the shell and Python script checks directly when changing files under
`scripts/`:

```bash
bash scripts/ci_check_shell_scripts.sh
PYTHONPATH=scripts uv run --no-config --no-project --managed-python --python 3.12 -- \
  python -m unittest \
  scripts.tests.test_format_benchmark_summary \
  scripts.tests.test_compare_benchmark_summary
```

## Source build with another Python version

Install the interpreter first, then use the same version for both environment
variables:

```bash
uv python install 3.13
PYO3_PYTHON="$(uv python find --no-config --no-project --managed-python 3.13)" \
R2X_PYTHON_VERSION=3.13 \
cargo build --release -p r2x --bins
```

The installed binary accepts Python versions with the same major.minor ABI as
the one used to build it. A patch version such as `3.12.1` is compatible with
a binary built against Python 3.12.

## Troubleshooting

### PyO3 cannot find Python

Set `PYO3_PYTHON` to a managed interpreter and set the matching
`R2X_PYTHON_VERSION`:

```bash
uv python find --no-config --no-project --managed-python 3.12
```

If the command does not return a path, install the interpreter with
`uv python install 3.12`.

### Python ABI mismatch

A configured runtime must use the same major.minor ABI as the binary. After
changing the configured version, recreate the managed environment:

```bash
r2x venv create --yes
```

### The command is not found

Confirm that the installation directory is on `PATH`. For a Cargo install,
that is usually `~/.cargo/bin`:

```bash
command -v r2x
```

### The runtime payload is missing

The `r2x-runtime` executable must remain beside the `r2x` command launcher. Re-run the
Cargo installation or copy both binaries together.

### Older Linux systems

The release Linux artifacts target glibc 2.28 or newer. Build from source when
the host uses an older glibc version or when a release artifact does not match
the host.
