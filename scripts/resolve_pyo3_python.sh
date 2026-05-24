#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/python_version.sh
source "$script_dir/python_version.sh"

requested_version="${R2X_PYTHON_VERSION:-}"
default_version="${R2X_DEFAULT_PYTHON_VERSION:-3.12}"

detect_python_version() {
    local python="$1"
    "$python" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")'
}

if [[ -n "${PYO3_PYTHON:-}" ]]; then
    if [[ -x "$PYO3_PYTHON" ]]; then
        python_version="$(detect_python_version "$PYO3_PYTHON" 2>/dev/null || true)"
        if [[ -z "$python_version" ]]; then
            echo "PYO3_PYTHON is set but did not report a Python version: $PYO3_PYTHON" >&2
            exit 1
        fi
        python_abi="$(r2x_python_abi_version "PYO3_PYTHON" "$python_version")"
        if [[ -n "$requested_version" ]]; then
            requested_abi="$(r2x_python_abi_version "R2X_PYTHON_VERSION" "$requested_version")"
            if [[ "$python_abi" != "$requested_abi" ]]; then
                echo "PYO3_PYTHON resolves to Python $python_abi but R2X_PYTHON_VERSION requests $requested_abi" >&2
                exit 1
            fi
        fi
        echo "$PYO3_PYTHON"
        exit 0
    fi
    echo "PYO3_PYTHON is set but not executable: $PYO3_PYTHON" >&2
    exit 1
fi

if [[ -n "$requested_version" ]]; then
    r2x_validate_python_version "R2X_PYTHON_VERSION" "$requested_version"
    if python_bin=$(r2x_find_uv_python "$requested_version"); then
        echo "$python_bin"
        exit 0
    fi
    install_hint="$(r2x_python_install_hint "$requested_version")"
    find_hint="$(r2x_python_find_hint "$requested_version")"
    echo "Requested R2X_PYTHON_VERSION=$requested_version was not found." >&2
    echo "Install it with: $install_hint" >&2
    echo "Verify with: $find_hint" >&2
    exit 1
fi

r2x_validate_python_version "R2X_DEFAULT_PYTHON_VERSION" "$default_version"
for version in "$default_version" 3.12 3.11; do
    if python_bin=$(r2x_find_uv_python "$version"); then
        echo "$python_bin"
        exit 0
    fi
done

if command -v python3 >/dev/null 2>&1; then
    command -v python3
    exit 0
fi

default_install_hint="$(r2x_python_install_hint "$default_version")"
default_find_hint="$(r2x_python_find_hint "$default_version")"
echo "Unable to find Python for PyO3. Install uv and run: $default_install_hint" >&2
echo "Verify with: $default_find_hint" >&2
exit 1
