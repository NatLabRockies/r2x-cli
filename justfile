CARGO := "cargo"
R2X_PKG := "r2x"
R2X_BIN := "target/debug/r2x"
PYTHON_VERSION := env_var_or_default("R2X_PYTHON_VERSION", "3.12")

# Auto-detect Python for PyO3 builds
export PYO3_PYTHON := shell('uv python find --no-config --no-project --managed-python "$1"', PYTHON_VERSION)
PYTHON_PREFIX := shell('dirname "$(dirname "$1")"', PYO3_PYTHON)

prepare-r2x:
	{{CARGO}} build -p {{R2X_PKG}} --bins

smoke-r2x: prepare-r2x
	{{R2X_BIN}} --help > /dev/null

fmt:
	{{CARGO}} fmt --all

clippy:
	{{CARGO}} clippy --workspace --all-targets --all-features

clippy-strict:
	{{CARGO}} clippy --workspace --all-targets --all-features -- -D warnings

clippy-fix:
	{{CARGO}} clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged

lint: fmt clippy

build:
	{{CARGO}} build --workspace --all-features

test:
	bash scripts/ci_run_with_python_lib_path.sh "{{PYTHON_PREFIX}}" {{CARGO}} test --workspace --all-features

run-reeds: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml reeds-test

run-s2p: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml s2p

run-r2p: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml r2p

venv: prepare-r2x
	{{R2X_BIN}} config venv create --yes

all: fmt clippy test
