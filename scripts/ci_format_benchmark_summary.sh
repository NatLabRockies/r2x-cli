#!/usr/bin/env bash
set -euo pipefail

main() {
    local input_path="${1:?Usage: ci_format_benchmark_summary.sh <input> <output>}"
    local output_path="${2:?Usage: ci_format_benchmark_summary.sh <input> <output>}"

    python3 scripts/format_benchmark_summary.py \
        --input "${input_path}" \
        --output "${output_path}" \
        --append-summary
}

main "$@"
