#!/usr/bin/env bash
set -euo pipefail

readonly shellcheck_version="${SHELLCHECK_VERSION:-v0.11.0}"
readonly install_dir="${1:-${RUNNER_TEMP:-/tmp}/shellcheck}"

platform_asset() {
    case "$(uname -s)-$(uname -m)" in
        Linux-x86_64)
            printf 'linux.x86_64\n'
            ;;
        Linux-aarch64 | Linux-arm64)
            printf 'linux.aarch64\n'
            ;;
        Darwin-x86_64)
            printf 'darwin.x86_64\n'
            ;;
        Darwin-arm64)
            printf 'darwin.aarch64\n'
            ;;
        *)
            printf 'unsupported platform for ShellCheck binary: %s-%s\n' "$(uname -s)" "$(uname -m)" >&2
            return 1
            ;;
    esac
}

main() {
    if command -v shellcheck >/dev/null 2>&1; then
        shellcheck --version
        return 0
    fi

    local asset
    asset="$(platform_asset)"

    local version_no_prefix="${shellcheck_version#v}"
    local archive_name="shellcheck-${version_no_prefix}.${asset}.tar.xz"
    local url="https://github.com/koalaman/shellcheck/releases/download/${shellcheck_version}/${archive_name}"

    mkdir -p "${install_dir}"
    curl --fail --silent --show-error --location --output "${install_dir}/${archive_name}" "${url}"
    tar -xJf "${install_dir}/${archive_name}" -C "${install_dir}"

    local bin_dir="${install_dir}/shellcheck-${version_no_prefix}"
    if [[ ! -x "${bin_dir}/shellcheck" ]]; then
        printf 'downloaded ShellCheck binary is not executable: %s\n' "${bin_dir}/shellcheck" >&2
        return 1
    fi

    if [[ -n "${GITHUB_PATH:-}" ]]; then
        printf '%s\n' "${bin_dir}" >>"${GITHUB_PATH}"
    fi

    "${bin_dir}/shellcheck" --version
}

main "$@"
