#!/usr/bin/env bash

# Prepare a minimal CPython shared library for the current host by leveraging UV's managed
# installations. The resulting file is copied into python-shim/<target>/ so we can later
# bundle it with release artifacts and point PyO3 at a stable path.

set -euo pipefail

if ! command -v uv >/dev/null 2>&1; then
    echo "error: uv is required to fetch the managed Python interpreter" >&2
    exit 1
fi

PY_VERSION="${PY_VERSION:-3.12}"
TARGET_TRIPLE="${TARGET_TRIPLE:-$(rustc -vV | sed -n 's/^host: //p')}"
OUT_DIR="python-shim/${TARGET_TRIPLE}"
SHIM_DIST_DIR="python-shim/dist"

mkdir -p "${OUT_DIR}"
rm -rf "${SHIM_DIST_DIR}"
mkdir -p "${SHIM_DIST_DIR}"

echo "Ensuring Python ${PY_VERSION} is installed via uv..."
uv python install "${PY_VERSION}" >/dev/null

# Use 'uv run python' to get the Python executable and base prefix
# This matches the approach used by the CLI application itself
echo "Detecting Python ${PY_VERSION} via uv..."
UV_PYTHON_INFO=$(uv run --python "${PY_VERSION}" python -c "import sys; print(sys.executable); print(sys.base_prefix)" 2>/dev/null)

if [[ -z "${UV_PYTHON_INFO}" ]]; then
    echo "error: failed to detect Python via 'uv run python'" >&2
    echo "Make sure Python ${PY_VERSION} is installed via uv" >&2
    exit 1
fi

# Parse the output: first line is executable, second line is base prefix
PYTHON_BIN=$(echo "${UV_PYTHON_INFO}" | head -n 1)
PY_PREFIX=$(echo "${UV_PYTHON_INFO}" | tail -n 1)

if [[ ! -x "${PYTHON_BIN}" ]]; then
    echo "error: Python executable not found or not executable: ${PYTHON_BIN}" >&2
    exit 1
fi

if [[ -z "${PY_PREFIX}" ]]; then
    echo "error: failed to resolve python prefix" >&2
    exit 1
fi

echo "Using Python binary: ${PYTHON_BIN}"
echo "Resolved Python prefix: ${PY_PREFIX}"

PY_SUFFIX="$("${PYTHON_BIN}" - <<PY
import sys
version = "${PY_VERSION}"
parts = version.split(".")
major = parts[0]
minor = parts[1] if len(parts) > 1 else "0"
print(f"{major}.{minor}")
PY
)"

declare -a SHIM_NAMES=("libpython${PY_SUFFIX}.dylib" "libpython${PY_SUFFIX}.so" "python${PY_SUFFIX/.}.dll")

case "${TARGET_TRIPLE}" in
    *apple-darwin*)
        LIB_NAME="libpython${PY_SUFFIX}.dylib"
        SRC_PATH="${PY_PREFIX}/lib/${LIB_NAME}"
        ;;
    *unknown-linux-gnu*|*unknown-linux-musl*)
        LIB_NAME="libpython${PY_SUFFIX}.so.1.0"
        SRC_PATH="${PY_PREFIX}/lib/libpython${PY_SUFFIX}.so"
        ;;
    *pc-windows-msvc*|*windows-gnu*)
        CLEAN_SUFFIX="${PY_SUFFIX/.}"
        LIB_NAME="python${CLEAN_SUFFIX}.dll"
        SRC_PATH="${PY_PREFIX}/python${CLEAN_SUFFIX}.dll"
        if [[ ! -f "${SRC_PATH}" ]]; then
            SRC_PATH="${PY_PREFIX}/DLLs/${LIB_NAME}"
        fi
        ;;
    *)
        echo "error: unsupported target triple '${TARGET_TRIPLE}'" >&2
        exit 1
        ;;
esac

if [[ ! -f "${SRC_PATH}" ]]; then
    echo "error: expected shared library not found at ${SRC_PATH}" >&2
    exit 1
fi

echo "Copying Python library as ${LIB_NAME}..."
cp "${SRC_PATH}" "${OUT_DIR}/${LIB_NAME}"
cp "${SRC_PATH}" "${SHIM_DIST_DIR}/${LIB_NAME}"

if [[ "${TARGET_TRIPLE}" == *"apple-darwin"* ]]; then
    install_name_tool -id "@rpath/${LIB_NAME}" "${OUT_DIR}/${LIB_NAME}"
fi



for shim_name in "${SHIM_NAMES[@]}"; do
    if [[ ! -f "${SHIM_DIST_DIR}/${shim_name}" ]]; then
        : > "${SHIM_DIST_DIR}/${shim_name}"
    fi
done

cat <<EOF
Copied ${LIB_NAME} into ${OUT_DIR}
Next steps:
  - Update build configuration to link PyO3 against ${OUT_DIR}
  - Package ${LIB_NAME} alongside release artifacts and patch rpaths/install names
EOF
