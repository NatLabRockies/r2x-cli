#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly script_dir
repo_root="$(cd "${script_dir}/.." && pwd)"
readonly repo_root

main() {
    local -a scripts=()
    while IFS= read -r -d '' script; do
        scripts+=("${script}")
    done < <(find "${repo_root}/scripts" -maxdepth 1 -type f -name '*.sh' -print0 | sort -z)

    if ((${#scripts[@]} == 0)); then
        printf 'No shell scripts found under %s\n' "${repo_root}/scripts"
        return 0
    fi

    local script
    for script in "${scripts[@]}"; do
        bash -n "${script}"
    done

    local shellcheck_bin="${SHELLCHECK_BIN:-shellcheck}"
    if command -v "${shellcheck_bin}" >/dev/null 2>&1; then
        "${shellcheck_bin}" "${scripts[@]}"
    else
        printf 'ShellCheck not found; ran bash syntax checks only.\n' >&2
    fi
}

main "$@"
