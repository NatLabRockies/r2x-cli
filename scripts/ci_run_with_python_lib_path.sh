#!/usr/bin/env bash
set -euo pipefail

main() {
    local python_prefix="${1:?Usage: ci_run_with_python_lib_path.sh <python-prefix> <command> [args...]}"
    shift

    if (($# == 0)); then
        printf 'missing command to run\n' >&2
        return 1
    fi

    # Rust test binaries embed Python rather than starting it through uv.
    export PYTHONHOME="${python_prefix}"

    case "$(uname -s)" in
        Darwin)
            export DYLD_LIBRARY_PATH="${python_prefix}/lib${DYLD_LIBRARY_PATH:+:${DYLD_LIBRARY_PATH}}"
            ;;
        Linux)
            export LD_LIBRARY_PATH="${python_prefix}/lib${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
            ;;
        MINGW* | MSYS* | CYGWIN*)
            export PATH="${python_prefix}${PATH:+:${PATH}}"
            ;;
    esac

    "$@"
}

main "$@"
