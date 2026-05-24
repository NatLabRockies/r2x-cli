#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/python_version.sh
source "$script_dir/python_version.sh"

python_bin="${PYO3_PYTHON:-}"
build_target="${CARGO_BUILD_TARGET:-${TARGET:-default}}"
diagnostic_status="ok"
diagnostic_error=""
python_version=""
python_executable=""
python_prefix=""
python_abi=""
python_libdir=""
requested_python_abi=""

collect_python_diagnostics() {
    if [[ -z "$python_bin" ]]; then
        diagnostic_status="error"
        diagnostic_error="PYO3_PYTHON is unset"
        return 1
    fi

    if [[ ! -x "$python_bin" ]]; then
        diagnostic_status="error"
        diagnostic_error="PYO3_PYTHON is not executable: $python_bin"
        return 1
    fi

    python_version="$("$python_bin" --version 2>&1 || true)"
    python_executable="$("$python_bin" -c 'import sys; print(sys.executable)' 2>/dev/null || true)"
    python_prefix="$("$python_bin" -c 'import sys; print(sys.prefix)' 2>/dev/null || true)"
    python_abi="$("$python_bin" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")' 2>/dev/null || true)"
    python_libdir="$("$python_bin" -c 'import sysconfig; print(sysconfig.get_config_var("LIBDIR") or "")' 2>/dev/null || true)"

    if [[ -z "$python_abi" ]]; then
        diagnostic_status="error"
        diagnostic_error="PYO3_PYTHON did not report a Python ABI version: $python_bin"
        return 1
    fi

    if ! diagnostic_error="$(r2x_python_abi_version "PYO3_PYTHON" "$python_abi" 2>&1 >/dev/null)"; then
        diagnostic_status="error"
        return 1
    fi
    diagnostic_error=""

    if [[ -n "${R2X_PYTHON_VERSION:-}" ]]; then
        if ! requested_python_abi="$(r2x_python_abi_version "R2X_PYTHON_VERSION" "$R2X_PYTHON_VERSION" 2>&1)"; then
            diagnostic_status="error"
            diagnostic_error="$requested_python_abi"
            return 1
        fi
        if [[ "$requested_python_abi" != "$python_abi" ]]; then
            diagnostic_status="error"
            diagnostic_error="R2X_PYTHON_VERSION requests Python ABI $requested_python_abi but PYO3_PYTHON reports $python_abi"
            return 1
        fi
    fi
}

emit_table() {
    local summary_path="$1"
    {
        echo "### PyO3 build context"
        echo
        echo "| Setting | Value |"
        echo "| --- | --- |"
        echo "| PYO3_PYTHON | \`${python_bin:-unset}\` |"
        echo "| PYO3_CONFIG_FILE | \`${PYO3_CONFIG_FILE:-unset}\` |"
        echo "| PYO3_CROSS | \`${PYO3_CROSS:-unset}\` |"
        echo "| PYO3_CROSS_LIB_DIR | \`${PYO3_CROSS_LIB_DIR:-unset}\` |"
        echo "| PYO3_CROSS_PYTHON_VERSION | \`${PYO3_CROSS_PYTHON_VERSION:-unset}\` |"
        echo "| R2X_PYTHON_VERSION | \`${R2X_PYTHON_VERSION:-unset}\` |"
        if [[ -n "$requested_python_abi" ]]; then
            echo "| requested Python ABI | \`${requested_python_abi}\` |"
        fi
        echo "| status | \`${diagnostic_status}\` |"
        if [[ -n "$diagnostic_error" ]]; then
            echo "| error | \`${diagnostic_error}\` |"
        fi
        echo "| rustc | \`$(rustc --version 2>/dev/null || echo unavailable)\` |"
        echo "| cargo | \`$(cargo --version 2>/dev/null || echo unavailable)\` |"
        echo "| cargo target | \`${build_target}\` |"
        if [[ "$diagnostic_status" == "ok" ]]; then
            echo "| Python version | \`${python_version}\` |"
            echo "| Python executable | \`${python_executable}\` |"
            echo "| Python prefix | \`${python_prefix}\` |"
            echo "| Python ABI | \`${python_abi}\` |"
            echo "| LIBDIR | \`${python_libdir}\` |"
        else
            echo "| Python version | \`unavailable\` |"
        fi
    } >> "$summary_path"
}

collect_python_diagnostics || true

if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    emit_table "$GITHUB_STEP_SUMMARY"
fi

if [[ "$diagnostic_status" == "ok" ]]; then
    echo "PyO3 Python: $python_bin ($python_version)"
else
    echo "PyO3 Python: ${diagnostic_error}"
fi
echo "R2X_PYTHON_VERSION: ${R2X_PYTHON_VERSION:-unset}"
echo "rustc: $(rustc --version 2>/dev/null || echo unavailable)"
echo "cargo: $(cargo --version 2>/dev/null || echo unavailable)"

if [[ "$diagnostic_status" != "ok" ]]; then
    exit 1
fi
