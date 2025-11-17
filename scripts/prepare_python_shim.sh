#!/usr/bin/env bash

# Prepare a minimal CPython shared library for the current host by leveraging UV's managed
# installations. The resulting file is copied into python-shim/<target>/ so we can later
# bundle it with release artifacts and point PyO3 at a stable path.

set -euo pipefail

if ! command -v uv >/dev/null 2>&1; then
    echo "error: uv is required to fetch the managed Python interpreter" >&2
    exit 1
fi

PYTHON_BIN="${PYTHON_BIN:-python3}"
if ! command -v "${PYTHON_BIN}" >/dev/null 2>&1; then
    if command -v python >/dev/null 2>&1; then
        PYTHON_BIN="python"
    else
        echo "error: python3/python not found" >&2
        exit 1
    fi
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

if [[ ! -f "scripts/detect_uv_python.py" ]]; then
    echo "error: scripts/detect_uv_python.py not found" >&2
    exit 1
fi

PY_PREFIX="$(PY_VERSION="${PY_VERSION}" "${PYTHON_BIN}" scripts/detect_uv_python.py)"

if [[ -z "${PY_PREFIX}" ]]; then
    echo "error: failed to resolve python prefix from uv output" >&2
    exit 1
fi

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
        LIB_NAME="libpython${PY_SUFFIX}.so"
        SRC_PATH="${PY_PREFIX}/lib/${LIB_NAME}"
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
