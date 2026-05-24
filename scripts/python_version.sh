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
