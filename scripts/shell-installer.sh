#!/bin/bash
# Custom shell installer for r2x-cli that includes extra files (e.g., libpython3.12.dylib)
set -euo pipefail

# Install uv if not present (Unix)
if ! command -v uv >/dev/null 2>&1; then
    echo "Installing uv..."
    curl -LsSf https://astral.sh/uv/install.sh | sh
    export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"  # Add to PATH for immediate use
else
    echo "uv already installed, skipping uv installation."
fi

# Verify uv
if ! command -v uv >/dev/null 2>&1; then
    echo "Error: uv installation failed. Please install manually from https://astral.sh/uv" >&2
    exit 1
fi

# Installation variables
INSTALL_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"  # Matches default: ~/.cargo/bin
TMPDIR="$(mktemp -d)"
ARCHIVE_URL="$1"  # Passed by dist as an argument (the .tar.xz URL)

echo "Installing r2x-cli to $INSTALL_DIR..."

# Download and extract the archive
curl -L "$ARCHIVE_URL" | tar -xz -C "$TMPDIR"

# Ensure install dir exists
mkdir -p "$INSTALL_DIR"

# Copy ALL extracted files (binary + included libs) to install dir
# This places r2x and libpython3.12.dylib side-by-side in ~/.cargo/bin/
cp -r "$TMPDIR"/* "$INSTALL_DIR/"

# Optional: Set executable permissions (cargo-dist handles this, but good to ensure)
chmod +x "$INSTALL_DIR/r2x"

echo "Installation complete! Run 'r2x' (it's in your PATH via ~/.cargo/bin)."
