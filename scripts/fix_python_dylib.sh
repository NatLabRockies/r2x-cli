#!/bin/bash
# Fix hardcoded Python library paths in r2x binaries
#
# This script fixes the issue where PyO3 embeds absolute paths to libpython
# at compile-time (e.g., /Users/runner/.local/share/uv/python/.../libpython3.12.dylib).
#
# On macOS: Uses install_name_tool to convert to @rpath-relative paths
# On Linux: Uses patchelf to set appropriate rpath
#
# Usage: ./scripts/fix_python_dylib.sh <binary_path>

set -euo pipefail

BINARY="${1:-}"

if [[ -z "$BINARY" ]]; then
    echo "Usage: $0 <binary_path>"
    echo "Example: $0 target/debug/r2x"
    exit 1
fi

if [[ ! -f "$BINARY" ]]; then
    echo "Error: Binary not found: $BINARY"
    exit 1
fi

fix_macos() {
    local binary="$1"

    echo "Fixing Python dylib paths for macOS binary: $binary"

    # Find the libpython reference
    local python_lib
    python_lib=$(otool -L "$binary" | grep -o '/.*libpython[0-9.]*\.dylib' | head -1 || true)

    if [[ -z "$python_lib" ]]; then
        echo "No libpython reference found in binary. Checking for other python libs..."
        python_lib=$(otool -L "$binary" | grep -o '/.*python.*\.dylib' | head -1 || true)
    fi

    if [[ -z "$python_lib" ]]; then
        echo "No Python library reference found. Binary may be statically linked or already fixed."
        return 0
    fi

    echo "Found Python library reference: $python_lib"

    # Extract the library filename (e.g., libpython3.12.dylib)
    local lib_name
    lib_name=$(basename "$python_lib")

    # Convert to @rpath-relative path
    local new_path="@rpath/$lib_name"

    echo "Converting: $python_lib -> $new_path"

    install_name_tool -change "$python_lib" "$new_path" "$binary"

    # Add common rpath locations
    # These allow the binary to find libpython in common installation locations

    # User's uv-managed Python (relative to home)
    install_name_tool -add_rpath "@executable_path/../lib" "$binary" 2>/dev/null || true

    # System Python locations
    install_name_tool -add_rpath "/usr/local/lib" "$binary" 2>/dev/null || true
    install_name_tool -add_rpath "/opt/homebrew/lib" "$binary" 2>/dev/null || true

    # Verify the change
    echo ""
    echo "Verification - Python library references after fix:"
    otool -L "$binary" | grep -i python || echo "(no python references)"

    echo ""
    echo "rpath entries:"
    otool -l "$binary" | grep -A2 LC_RPATH | grep path || echo "(no rpath entries)"

    echo ""
    echo "Done! Binary fixed: $binary"
    echo ""
    echo "NOTE: Users must have Python installed and libpython accessible via:"
    echo "  - @executable_path/../lib"
    echo "  - /usr/local/lib"
    echo "  - /opt/homebrew/lib"
    echo "  - Or set DYLD_LIBRARY_PATH to Python's lib directory"
}

fix_linux() {
    local binary="$1"

    echo "Fixing Python library paths for Linux binary: $binary"

    # Check if patchelf is available
    if ! command -v patchelf &> /dev/null; then
        echo "Error: patchelf is required but not installed."
        echo "Install with: dnf install -y epel-release patchelf  # (RHEL/Rocky)"
        echo "         or: apt-get install -y patchelf  # (Debian/Ubuntu)"
        exit 1
    fi

    # Find current Python library reference
    local python_lib
    python_lib=$(ldd "$binary" 2>/dev/null | grep -o '/.*libpython[0-9.]*\.so[0-9.]*' | head -1 || true)

    if [[ -z "$python_lib" ]]; then
        echo "No libpython reference found in binary."
        return 0
    fi

    echo "Found Python library reference: $python_lib"

    # Set rpath to include common Python library locations
    # $ORIGIN allows finding libs relative to the binary
    local new_rpath='$ORIGIN/../lib:$ORIGIN:$ORIGIN/../lib/python3.12/config-3.12-x86_64-linux-gnu:/usr/local/lib:/usr/lib:/usr/lib64'

    echo "Setting rpath to: $new_rpath"

    patchelf --set-rpath "$new_rpath" "$binary"

    # Verify
    echo ""
    echo "Verification - rpath after fix:"
    patchelf --print-rpath "$binary"

    echo ""
    echo "Dynamic libraries:"
    ldd "$binary" | grep -i python || echo "(no python references - may use dlopen)"

    echo ""
    echo "Done! Binary fixed: $binary"
    echo ""
    echo "NOTE: Users must have Python installed with libpython accessible via:"
    echo "  - \$ORIGIN/../lib (relative to binary)"
    echo "  - /usr/local/lib"
    echo "  - /usr/lib or /usr/lib64"
    echo "  - Or set LD_LIBRARY_PATH to Python's lib directory"
}

# Call the appropriate fix function based on platform
case "$(uname -s)" in
    Darwin)
        fix_macos "$BINARY"
        ;;
    Linux)
        fix_linux "$BINARY"
        ;;
    *)
        echo "Unsupported platform: $(uname -s)"
        exit 1
        ;;
esac
