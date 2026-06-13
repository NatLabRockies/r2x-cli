#!/usr/bin/env bash
set -euo pipefail

append_env() {
    local key="$1"
    local value="$2"

    if [[ -n "${GITHUB_ENV:-}" ]]; then
        printf '%s=%s\n' "${key}" "${value}" >>"${GITHUB_ENV}"
    fi
}

append_no_baseline_summary() {
    if [[ -z "${GITHUB_STEP_SUMMARY:-}" ]]; then
        return 0
    fi

    {
        printf '\n### Plugin Benchmark Delta\n\n'
        printf "No baseline artifact found from recent successful \`main\` runs.\n"
    } >>"${GITHUB_STEP_SUMMARY}"
}

main() {
    local baseline_dir="${1:?Usage: ci_download_benchmark_baseline.sh <baseline-dir> <repository> <current-run-id> <server-url>}"
    local repository="${2:?Usage: ci_download_benchmark_baseline.sh <baseline-dir> <repository> <current-run-id> <server-url>}"
    local current_run_id="${3:?Usage: ci_download_benchmark_baseline.sh <baseline-dir> <repository> <current-run-id> <server-url>}"
    local server_url="${4:?Usage: ci_download_benchmark_baseline.sh <baseline-dir> <repository> <current-run-id> <server-url>}"

    mkdir -p "${baseline_dir}"

    local -a run_ids=()
    mapfile -t run_ids < <(
        gh api "/repos/${repository}/actions/workflows/build.yml/runs" \
            -f branch=main \
            -f status=success \
            -f per_page=30 \
            --jq ".workflow_runs[] | select(.id != ${current_run_id}) | .id"
    )

    local selected_run=""
    local run_id
    for run_id in "${run_ids[@]}"; do
        if gh run download "${run_id}" -n r2x-plugin-benchmark-summary -D "${baseline_dir}" >/dev/null 2>&1; then
            if [[ -f "${baseline_dir}/r2x-plugin-benchmark.txt" ]]; then
                selected_run="${run_id}"
                break
            fi
        fi
    done

    if [[ -n "${selected_run}" ]]; then
        append_env "BASELINE_BENCHMARK_PATH" "${baseline_dir}/r2x-plugin-benchmark.txt"
        append_env "BASELINE_BENCHMARK_RUN_ID" "${selected_run}"
        append_env "BASELINE_BENCHMARK_RUN_URL" "${server_url}/${repository}/actions/runs/${selected_run}"
        printf 'Using baseline benchmark artifact from run %s.\n' "${selected_run}"
    else
        printf 'No baseline benchmark artifact found from recent successful main runs.\n'
        append_no_baseline_summary
    fi
}

main "$@"
