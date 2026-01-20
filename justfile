CARGO := "cargo"
R2X_PKG := "r2x"
R2X_BIN := "target/debug/r2x"

# Build r2x and patch dylib lookup on macOS
prepare-r2x:
	{{CARGO}} build -p {{R2X_PKG}}
	if [ "$(uname)" = "Darwin" ]; then install_name_tool -change @rpath/libiconv.2.dylib /usr/lib/libiconv.2.dylib {{R2X_BIN}}; fi

# Ensure r2x launches without dylib loader errors
smoke-r2x: prepare-r2x
	{{R2X_BIN}} --help > /dev/null

# Format sources
fmt:
	{{CARGO}} fmt --all

# Run Clippy with strict workspace coverage
clippy:
	{{CARGO}} clippy --workspace --all-targets

# Run formatting + linting
lint: fmt clippy

# Build all crates and features
build:
	{{CARGO}} build --workspace --all-features

# Run Rust tests for all crates and features
test:
	{{CARGO}} test --workspace --all-features

# Run the ReEDS pipeline
run-reeds: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml reeds-test

# Run the Sienna to Plexos pipeline
run-s2p: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml s2p

# Run the ReEDS to Plexos pipeline
run-r2p: prepare-r2x
	{{R2X_BIN}} run pipeline.yaml r2p

# Provision the Python venv
venv: prepare-r2x
	{{R2X_BIN}} config venv create --yes

# Validate the Python bridge
python-bridge: prepare-r2x
	{{R2X_BIN}} config venv create --yes
	{{R2X_BIN}} run plugin -vv r2x-sienna.parser json_path=sienna_old.json
