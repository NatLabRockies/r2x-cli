#!/usr/bin/env bash
set -euo pipefail

main() {
    local baseline_path="${1:?Usage: ci_compare_benchmark_baseline.sh <baseline> <current> <output> <baseline-run-id> <baseline-run-url>}"
    local current_path="${2:?Usage: ci_compare_benchmark_baseline.sh <baseline> <current> <output> <baseline-run-id> <baseline-run-url>}"
    local output_path="${3:?Usage: ci_compare_benchmark_baseline.sh <baseline> <current> <output> <baseline-run-id> <baseline-run-url>}"
    local baseline_run_id="${4:?Usage: ci_compare_benchmark_baseline.sh <baseline> <current> <output> <baseline-run-id> <baseline-run-url>}"
    local baseline_run_url="${5:?Usage: ci_compare_benchmark_baseline.sh <baseline> <current> <output> <baseline-run-id> <baseline-run-url>}"

    local -a threshold_args=()
    if [[ -n "${R2X_BENCHMARK_REGRESSION_PCT:-}" ]]; then
        threshold_args+=(--fail-on-regression-pct "${R2X_BENCHMARK_REGRESSION_PCT}")
    fi

    uv run --no-config --no-project --managed-python \
        --python "${R2X_PYTHON_VERSION:-3.12}" -- \
        python scripts/compare_benchmark_summary.py \
        --baseline "${baseline_path}" \
        --current "${current_path}" \
        --baseline-run-id "${baseline_run_id}" \
        --baseline-run-url "${baseline_run_url}" \
        --output "${output_path}" \
        "${threshold_args[@]}" \
        --print-status-line \
        --write-github-output \
        --append-summary
}

main "$@"
