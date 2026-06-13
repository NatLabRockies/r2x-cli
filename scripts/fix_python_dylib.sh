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

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/python_version.sh
source "$script_dir/python_version.sh"

# Add rpath entry, ignoring "already exists" errors
add_rpath() {
    install_name_tool -add_rpath "$1" "$2" 2>/dev/null || true
}

python_abi_version() {
    r2x_python_abi_version "PYTHON_VERSION" "$1"
}

detect_pyo3_python_abi() {
    local python_bin="$1"

    if [[ ! -x "$python_bin" ]]; then
        echo "PYO3_PYTHON is set but not executable: $python_bin" >&2
        return 1
    fi

    local python_version
    python_version=$("$python_bin" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")' 2>/dev/null || true)
    if [[ -z "$python_version" ]]; then
        echo "PYO3_PYTHON is set but did not report a Python version: $python_bin" >&2
        return 1
    fi

    r2x_python_abi_version "PYO3_PYTHON" "$python_version"
}

validate_pyo3_python_matches_configured_version() {
    local python_bin="${PYO3_PYTHON:-}"
    local configured_version="$1"

    if [[ -z "$configured_version" || -z "$python_bin" ]]; then
        return 0
    fi

    local requested_abi pyo3_abi
    requested_abi=$(r2x_python_abi_version "R2X_PYTHON_VERSION" "$configured_version")
    pyo3_abi=$(detect_pyo3_python_abi "$python_bin")

    if [[ "$pyo3_abi" != "$requested_abi" ]]; then
        echo "PYO3_PYTHON resolves to Python $pyo3_abi but R2X_PYTHON_VERSION requests $requested_abi" >&2
        return 1
    fi
}

resolve_uv_python_tag() {
    local python_bin="${PYO3_PYTHON:-}"
    local configured_version="${R2X_PYTHON_VERSION:-${PYTHON_VERSION:-}}"
    local python_version="${configured_version:-3.12}"

    if [[ -n "$configured_version" ]]; then
        r2x_validate_python_version "R2X_PYTHON_VERSION" "$configured_version"
        validate_pyo3_python_matches_configured_version "$configured_version"
    elif [[ -n "$python_bin" ]]; then
        detect_pyo3_python_abi "$python_bin" >/dev/null
    fi

    if [[ -z "$python_bin" ]] && command -v uv &> /dev/null; then
        if [[ -n "$configured_version" ]]; then
            python_bin=$(r2x_find_uv_python "$python_version" || true)
        else
            python_bin=$(r2x_find_uv_python "$python_version" || r2x_find_uv_python 3.11 || true)
        fi
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

resolve_python_version() {
    local python_bin="${PYO3_PYTHON:-}"
    local configured_version="${R2X_PYTHON_VERSION:-${PYTHON_VERSION:-}}"

    if [[ -n "$configured_version" ]]; then
        r2x_validate_python_version "R2X_PYTHON_VERSION" "$configured_version"
        validate_pyo3_python_matches_configured_version "$configured_version"
        python_abi_version "$configured_version"
        return 0
    fi

    if [[ -z "$python_bin" ]] && command -v uv &> /dev/null; then
        python_bin=$(uv python find 3.12 2>/dev/null || uv python find 3.11 2>/dev/null || true)
    fi

    if [[ -n "$python_bin" && -x "$python_bin" ]]; then
        detect_pyo3_python_abi "$python_bin"
        return 0
    fi

    echo "3.12"
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

    # Check for ANY libpython reference (resolved or "=> not found")
    local python_refs
    python_refs=$(ldd "$binary" 2>/dev/null | grep -i 'libpython' || true)

    if [[ -z "$python_refs" ]]; then
        echo "No libpython reference found in binary."
        return 0
    fi

    # Log what ldd found
    local python_lib
    python_lib=$(echo "$python_refs" | grep -o '/.*libpython[0-9.]*\.so[0-9.]*' | head -1 || true)

    if [[ -n "$python_lib" ]]; then
        echo "Found: $python_lib"
    else
        echo "Found unresolved libpython reference (not in current search path):"
        echo "  $(echo "$python_refs" | head -1)"
    fi

    # $ORIGIN allows finding libs relative to the binary
    local uv_rpath=""
    local uv_tag
    if uv_tag=$(resolve_uv_python_tag); then
        uv_rpath=":\$ORIGIN/../share/uv/python/$uv_tag/lib"
    fi

    local python_version
    python_version=$(resolve_python_version)
    local new_rpath="\$ORIGIN/../lib:\$ORIGIN:\$ORIGIN/../lib/python${python_version}/config-${python_version}-x86_64-linux-gnu:/usr/local/lib:/usr/lib:/usr/lib64${uv_rpath}"

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

main() {
    local binary="${1:-}"

    if [[ -z "$binary" ]]; then
        echo "Usage: $0 <binary_path>"
        echo "Example: $0 target/debug/r2x"
        exit 1
    fi

    if [[ ! -f "$binary" ]]; then
        echo "Error: Binary not found: $binary"
        exit 1
    fi

    case "$(uname -s)" in
        Darwin)
            fix_macos "$binary"
            ;;
        Linux)
            fix_linux "$binary"
            ;;
        *)
            echo "Unsupported platform: $(uname -s)"
            exit 1
            ;;
    esac
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    main "$@"
fi
