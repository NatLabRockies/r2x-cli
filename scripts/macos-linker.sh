#!/usr/bin/env bash
set -euo pipefail

output=''
args=("$@")
for ((index = 0; index + 1 < ${#args[@]}; index++)); do
    if [[ "${args[index]}" == '-o' ]]; then
        output="${args[index + 1]}"
    fi
done

clang_bin="${R2X_MACOS_CLANG:-$(command -v clang)}"
"${clang_bin}" "${args[@]}"

if [[ -z "${output}" || ! -f "${output}" ]]; then
    exit 0
fi

python_library="$(otool -L "${output}" | awk '/libpython/ {print $1; exit}')"
if [[ -z "${python_library}" || "${python_library}" == '@rpath/'* ]]; then
    exit 0
fi

expected="@rpath/$(basename "${python_library}")"
install_name_tool -change "${python_library}" "${expected}" "${output}"

actual="$(otool -L "${output}" | awk '/libpython/ {print $1; exit}')"
if [[ "${actual}" != "${expected}" ]]; then
    printf 'Failed to make Python dependency relocatable in %s: %s\n' "${output}" "${actual}" >&2
    exit 1
fi
