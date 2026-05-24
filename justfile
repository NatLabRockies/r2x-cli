CARGO := "cargo"
R2X_PKG := "r2x"
R2X_BIN := "target/debug/r2x"
PYTHON_VERSION := env_var_or_default("R2X_PYTHON_VERSION", "3.12")

# Auto-detect Python for PyO3 builds
export PYO3_PYTHON := `R2X_DEFAULT_PYTHON_VERSION={{PYTHON_VERSION}} ./scripts/resolve_pyo3_python.sh`

prepare-r2x:
	{{CARGO}} build -p {{R2X_PKG}}
	if [ "$(uname)" = "Darwin" ]; then install_name_tool -change @rpath/libiconv.2.dylib /usr/lib/libiconv.2.dylib {{R2X_BIN}}; fi
	./scripts/fix_python_dylib.sh {{R2X_BIN}}

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
	{{CARGO}} test --workspace --all-features

run-reeds: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml reeds-test

run-s2p: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml s2p

run-r2p: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml r2p

venv: prepare-r2x
	{{R2X_BIN}} config venv create --yes

all: fmt clippy test
