#!/usr/bin/env bash
set -euo pipefail

main() {
    local python_prefix="${1:?Usage: ci_run_with_python_lib_path.sh <python-prefix> <command> [args...]}"
    shift

    if (($# == 0)); then
        printf 'missing command to run\n' >&2
        return 1
    fi

    export LD_LIBRARY_PATH="${python_prefix}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
    "$@"
}

main "$@"
