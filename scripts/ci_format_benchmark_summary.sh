#!/usr/bin/env bash
set -euo pipefail

main() {
    local input_path="${1:?Usage: ci_format_benchmark_summary.sh <input> <output>}"
    local output_path="${2:?Usage: ci_format_benchmark_summary.sh <input> <output>}"

    uv run --no-config --no-project --managed-python \
        --python "${R2X_PYTHON_VERSION:-3.12}" -- \
        python scripts/format_benchmark_summary.py \
        --input "${input_path}" \
        --output "${output_path}" \
        --append-summary
}

main "$@"
