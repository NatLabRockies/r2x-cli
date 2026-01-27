CARGO := "cargo"
R2X_PKG := "r2x"
R2X_BIN := "target/debug/r2x"

# Auto-detect Python for PyO3 builds
export PYO3_PYTHON := `uv python find 3.12 2>/dev/null || uv python find 3.11 2>/dev/null || which python3`

prepare-r2x:
	{{CARGO}} build -p {{R2X_PKG}}
	if [ "$(uname)" = "Darwin" ]; then install_name_tool -change @rpath/libiconv.2.dylib /usr/lib/libiconv.2.dylib {{R2X_BIN}}; fi
	./scripts/fix_python_dylib.sh {{R2X_BIN}}

smoke-r2x: prepare-r2x
	{{R2X_BIN}} --help > /dev/null

fmt:
	{{CARGO}} fmt --all

clippy:
	{{CARGO}} clippy --workspace --all-targets

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
