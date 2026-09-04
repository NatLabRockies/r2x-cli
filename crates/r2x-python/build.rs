fn main() {
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");
    println!("cargo:rerun-if-env-changed=R2X_PYTHON_VERSION");

    let pyo3_python = std::env::var("PYO3_PYTHON")
        .ok()
        .filter(|python| !python.trim().is_empty());
    if pyo3_python.is_none() {
        let requested = std::env::var("R2X_PYTHON_VERSION")
            .ok()
            .filter(|version| !version.trim().is_empty())
            .unwrap_or_else(|| "3.12".to_string());
        println!(
            "cargo:error=PYO3_PYTHON is required. Set it to a UV-managed interpreter with: uv python find --managed-python {requested}"
        );
        std::process::exit(1);
    }

    // Only r2x-config needs R2X_BUILD_PYTHON_VERSION; this crate
    // validates the build environment so the user catches a misconfigured
    // PYO3_PYTHON or R2X_PYTHON_VERSION early, before the actual build.
    if let Err(error) = r2x_build_support::detect_build_python_version() {
        println!("cargo:error={error}");
        std::process::exit(1);
    }
}
