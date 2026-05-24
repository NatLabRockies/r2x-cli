#!/usr/bin/env bash
set -euo pipefail

main() {
    local python_prefix="${1:?Usage: ci_fix_libpython_install_name.sh <python-prefix> <python-abi-version>}"
    local python_abi_version="${2:?Usage: ci_fix_libpython_install_name.sh <python-prefix> <python-abi-version>}"
    local libpython="${python_prefix}/lib/libpython${python_abi_version}.dylib"

    if [[ ! -f "${libpython}" ]]; then
        printf 'libpython not found at %s (skipping install name fix)\n' "${libpython}"
        return 0
    fi

    printf 'Fixing libpython install name for portable binary linking\n'
    printf '  Before: %s\n' "$(otool -D "${libpython}" | tail -1)"
    install_name_tool -id "@rpath/libpython${python_abi_version}.dylib" "${libpython}"
    printf '  After:  %s\n' "$(otool -D "${libpython}" | tail -1)"
}

main "$@"
