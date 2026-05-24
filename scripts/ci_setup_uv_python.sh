#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly script_dir
# shellcheck source=scripts/python_version.sh
source "${script_dir}/python_version.sh"

gha_error() {
    local message="$1"
    printf '::error::%s\n' "${message}" >&2
}

append_output() {
    local key="$1"
    local value="$2"

    if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
        printf '%s=%s\n' "${key}" "${value}" >>"${GITHUB_OUTPUT}"
    fi
}

append_env() {
    local key="$1"
    local value="$2"

    if [[ -n "${GITHUB_ENV:-}" ]]; then
        printf '%s=%s\n' "${key}" "${value}" >>"${GITHUB_ENV}"
    fi
}

append_summary() {
    local requested_version="$1"
    local requested_abi="$2"
    local resolved_query="$3"
    local python_version="$4"
    local python_abi="$5"
    local python_bin="$6"
    local python_prefix="$7"
    local uv_version="$8"

    if [[ -z "${GITHUB_STEP_SUMMARY:-}" ]]; then
        return 0
    fi

    {
        printf '### Python runtime\n\n'
        printf '| Setting | Value |\n'
        printf '| --- | --- |\n'
        printf "| Requested | \`%s\` |\n" "${requested_version}"
        printf "| Requested ABI | \`%s\` |\n" "${requested_abi}"
        printf "| Resolved query | \`%s\` |\n" "${resolved_query}"
        printf "| Resolved | \`%s\` |\n" "${python_version}"
        printf "| ABI | \`%s\` |\n" "${python_abi}"
        printf "| Interpreter | \`%s\` |\n" "${python_bin}"
        printf "| Prefix | \`%s\` |\n" "${python_prefix}"
        printf "| uv | \`%s\` |\n" "${uv_version}"
    } >>"${GITHUB_STEP_SUMMARY}"
}

install_requested_python() {
    local requested_version="$1"
    local requested_abi="$2"
    local install_hint="$3"

    if uv python install "${requested_version}"; then
        return 0
    fi

    if [[ "${requested_abi}" != "${requested_version}" ]]; then
        if uv python install "${requested_abi}"; then
            return 0
        fi
        gha_error "unable to install requested Python version ${requested_version} (fallback ABI ${requested_abi} also failed)"
    else
        gha_error "unable to install requested Python version ${requested_version}"
    fi

    gha_error "Install it with: ${install_hint}"
    return 1
}

resolve_python_bin() {
    local requested_version="$1"
    local requested_abi="$2"
    local -n resolved_query_ref="$3"

    resolved_query_ref="${requested_version}"
    local python_bin
    python_bin="$(uv python find "${resolved_query_ref}" 2>/dev/null || true)"

    if [[ -z "${python_bin}" && "${requested_abi}" != "${requested_version}" ]]; then
        resolved_query_ref="${requested_abi}"
        python_bin="$(uv python find "${resolved_query_ref}" 2>/dev/null || true)"
    fi

    printf '%s\n' "${python_bin}"
}

main() {
    local requested_version="${1:?Usage: ci_setup_uv_python.sh <python-version>}"
    local requested_abi
    if ! requested_abi="$(r2x_python_abi_version "python-version" "${requested_version}" 2>&1)"; then
        gha_error "${requested_abi}"
        return 1
    fi

    local install_hint find_hint
    install_hint="$(r2x_python_install_hint "${requested_version}")"
    find_hint="$(r2x_python_find_hint "${requested_version}")"

    install_requested_python "${requested_version}" "${requested_abi}" "${install_hint}"

    local resolved_query python_bin
    python_bin="$(resolve_python_bin "${requested_version}" "${requested_abi}" resolved_query)"
    if [[ -z "${python_bin}" ]]; then
        gha_error "unable to resolve Python for requested version ${requested_version} (tried ABI ${requested_abi})"
        gha_error "Install it with: ${install_hint}"
        gha_error "Verify with: ${find_hint}"
        return 1
    fi

    local python_prefix python_version python_abi uv_version
    python_prefix="$(dirname "$(dirname "${python_bin}")")"
    python_version="$("${python_bin}" --version)"
    python_abi="$("${python_bin}" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')"
    uv_version="$(uv --version)"

    if [[ "${python_abi}" != "${requested_abi}" ]]; then
        gha_error "python-version requested ABI ${requested_abi} but uv resolved ABI ${python_abi} at ${python_bin}"
        return 1
    fi

    append_env "PYO3_PYTHON" "${python_bin}"
    append_env "R2X_PYTHON_VERSION" "${requested_version}"
    append_output "python-path" "${python_bin}"
    append_output "resolved-version" "${python_version}"
    append_output "python-abi-version" "${python_abi}"
    append_output "python-prefix" "${python_prefix}"

    printf 'Setting PYO3_PYTHON => %s (%s)\n' "${python_bin}" "${python_version}"
    append_summary "${requested_version}" "${requested_abi}" "${resolved_query}" "${python_version}" "${python_abi}" "${python_bin}" "${python_prefix}" "${uv_version}"
}

main "$@"
