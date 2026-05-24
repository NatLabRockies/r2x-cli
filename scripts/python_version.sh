#!/usr/bin/env bash

r2x_python_abi_version() {
    local label="$1"
    local version="$2"

    if [[ ! "$version" =~ ^3\.([0-9]+)(\.[0-9]+)?$ ]]; then
        echo "$label must be a Python version like 3.12 or 3.12.1, got: $version" >&2
        return 1
    fi

    local minor="${BASH_REMATCH[1]}"
    if ((10#$minor < 11)); then
        echo "$label=$version is not supported; r2x requires Python 3.11 or newer" >&2
        return 1
    fi

    echo "3.${minor}"
}

r2x_validate_python_version() {
    local label="$1"
    local version="$2"

    r2x_python_abi_version "$label" "$version" >/dev/null
}

r2x_python_install_hint() {
    local requested_version="$1"
    local requested_abi=""

    if ! requested_abi="$(r2x_python_abi_version "R2X_PYTHON_VERSION" "$requested_version" 2>/dev/null)"; then
        echo "uv python install $requested_version"
        return 0
    fi

    if [[ "$requested_abi" != "$requested_version" ]]; then
        echo "uv python install $requested_version || uv python install $requested_abi"
    else
        echo "uv python install $requested_version"
    fi
}

r2x_python_find_hint() {
    local requested_version="$1"
    local requested_abi=""

    if ! requested_abi="$(r2x_python_abi_version "R2X_PYTHON_VERSION" "$requested_version" 2>/dev/null)"; then
        echo "uv python find $requested_version"
        return 0
    fi

    if [[ "$requested_abi" != "$requested_version" ]]; then
        echo "uv python find $requested_version || uv python find $requested_abi"
    else
        echo "uv python find $requested_version"
    fi
}

r2x_find_uv_python() {
    local requested_version="$1"

    command -v uv >/dev/null 2>&1 || return 1

    local python_bin=""
    python_bin="$(uv python find "$requested_version" 2>/dev/null || true)"
    if [[ -n "$python_bin" ]]; then
        echo "$python_bin"
        return 0
    fi

    local requested_abi=""
    if requested_abi="$(r2x_python_abi_version "R2X_PYTHON_VERSION" "$requested_version" 2>/dev/null)"; then
        if [[ "$requested_version" == *.*.* ]] && [[ "$requested_abi" != "$requested_version" ]]; then
            python_bin="$(uv python find "$requested_abi" 2>/dev/null || true)"
            if [[ -n "$python_bin" ]]; then
                echo "$python_bin"
                return 0
            fi
        fi
    fi

    return 1
}
