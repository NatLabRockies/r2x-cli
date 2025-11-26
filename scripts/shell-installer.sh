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
#curl -L "$ARCHIVE_URL" | tar -xz -C "$TMPDIR"

# Download and extract the archive
# Or install from local path (for testing)
if [[ "$ARCHIVE_URL" =~ ^https?:// ]]; then
    curl -L "$ARCHIVE_URL" | xz -d | tar -x -C "$TMPDIR"
else
    xz -d < "$ARCHIVE_URL" | tar -x -C "$TMPDIR"
fi


# Ensure install dir exists
mkdir -p "$INSTALL_DIR"

# Copy ALL extracted files (binary + included libs) to install dir
# This places r2x and libpython3.12.dylib side-by-side in ~/.cargo/bin/
# Copy only the r2x binary and Python library (dylib/so) to install dir
# This places r2x and the relevant libpython next to each other in ~/.cargo/bin/
subdir=$(find "$TMPDIR" -maxdepth 1 -type d -not -path "$TMPDIR" | head -n1)
if [ -n "$subdir" ]; then
    src_dir="$subdir"
else
    src_dir="$TMPDIR"
fi

# Detect OS and set library extension
if [[ "$(uname -s)" == "Darwin" ]]; then
    lib_ext="dylib"
else
    lib_ext="so"
fi

for file in "$src_dir"/*; do
    if [ -f "$file" ]; then
        basename=$(basename "$file")
        if [[ "$basename" == "r2x" ]] || [[ "$basename" =~ python && "$basename" =~ \.$lib_ext$ ]]; then
            cp "$file" "$INSTALL_DIR/"
        fi
    fi
done
# Optional: Set executable permissions (cargo-dist handles this, but good to ensure)
chmod +x "$INSTALL_DIR/r2x"

echo "Installation complete! Run 'r2x' (it's in your PATH via ~/.cargo/bin)."
