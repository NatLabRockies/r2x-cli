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

# Add rpath entry, ignoring "already exists" errors
add_rpath() {
    install_name_tool -add_rpath "$1" "$2" 2>/dev/null || true
}

resolve_uv_python_tag() {
    local python_bin="${PYO3_PYTHON:-}"

    if [[ -z "$python_bin" ]] && command -v uv &> /dev/null; then
        python_bin=$(uv python find 3.12 2>/dev/null || true)
    fi

    if [[ -z "$python_bin" ]]; then
        return 1
    fi

    local python_dir uv_tag
    python_dir=$(dirname "$python_bin")
    uv_tag=$(basename "$(dirname "$python_dir")")

    if [[ -z "$uv_tag" ]]; then
        return 1
    fi

    echo "$uv_tag"
}

fix_macos() {
    local binary="$1"

    echo "Fixing Python dylib paths for macOS binary: $binary"

    if otool -L "$binary" | grep -q '@rpath/libpython'; then
        echo "Python library already uses @rpath"
    else
        # Find the libpython reference (try specific pattern first, then broader)
        local python_lib
        python_lib=$(otool -L "$binary" | grep -o '/.*libpython[0-9.]*\.dylib' | head -1 || true)

        if [[ -z "$python_lib" ]]; then
            python_lib=$(otool -L "$binary" | grep -o '/.*python.*\.dylib' | head -1 || true)
        fi

        if [[ -z "$python_lib" ]]; then
            echo "No Python library reference found. Binary may be statically linked or already fixed."
            return 0
        fi

        echo "Found: $python_lib"

        local lib_name new_path
        lib_name=$(basename "$python_lib")
        new_path="@rpath/$lib_name"

        echo "Converting to: $new_path"
        install_name_tool -change "$python_lib" "$new_path" "$binary"
    fi

    # Add common rpath locations for finding libpython
    add_rpath "@executable_path/../lib" "$binary"

    local uv_tag
    if uv_tag=$(resolve_uv_python_tag); then
        add_rpath "@executable_path/../share/uv/python/$uv_tag/lib" "$binary"
    else
        case "$(uname -m)" in
            arm64)
                add_rpath "/opt/homebrew/lib" "$binary"
                ;;
            x86_64)
                add_rpath "/usr/local/lib" "$binary"
                ;;
            *)
                add_rpath "/usr/local/lib" "$binary"
                ;;
        esac
    fi

    # Verify
    echo ""
    echo "Python references after fix:"
    otool -L "$binary" | grep -i python || echo "  (none)"
    echo ""
    echo "rpath entries:"
    otool -l "$binary" | grep -A2 LC_RPATH | grep path || echo "  (none)"
    echo ""
    echo "Done! Users need libpython accessible via rpath or DYLD_LIBRARY_PATH."
}

fix_linux() {
    local binary="$1"

    echo "Fixing Python library paths for Linux binary: $binary"

    if ! command -v patchelf &> /dev/null; then
        echo "Error: patchelf is required but not installed."
        echo "  RHEL/Rocky: dnf install -y epel-release patchelf"
        echo "  Debian/Ubuntu: apt-get install -y patchelf"
        exit 1
    fi

    local python_lib
    python_lib=$(ldd "$binary" 2>/dev/null | grep -o '/.*libpython[0-9.]*\.so[0-9.]*' | head -1 || true)

    if [[ -z "$python_lib" ]]; then
        echo "No libpython reference found in binary."
        return 0
    fi

    echo "Found: $python_lib"

    # $ORIGIN allows finding libs relative to the binary
    local uv_rpath=""
    local uv_tag
    if uv_tag=$(resolve_uv_python_tag); then
        uv_rpath=":\$ORIGIN/../share/uv/python/$uv_tag/lib"
    fi

    local new_rpath='$ORIGIN/../lib:$ORIGIN:$ORIGIN/../lib/python3.12/config-3.12-x86_64-linux-gnu:/usr/local/lib:/usr/lib:/usr/lib64'"$uv_rpath"

    echo "Setting rpath to: $new_rpath"
    patchelf --set-rpath "$new_rpath" "$binary"

    # Verify
    echo ""
    echo "rpath after fix:"
    patchelf --print-rpath "$binary"
    echo ""
    echo "Python references:"
    ldd "$binary" | grep -i python || echo "  (none - may use dlopen)"
    echo ""
    echo "Done! Users need libpython accessible via rpath or LD_LIBRARY_PATH."
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
