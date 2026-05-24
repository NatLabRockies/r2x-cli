#[path = "../../scripts/build_python_version.rs"]
mod build_python_version;

fn main() {
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");
    println!("cargo:rerun-if-env-changed=R2X_PYTHON_VERSION");
    println!("cargo:rerun-if-changed=../../scripts/build_python_version.rs");

    // Only r2x-config needs R2X_BUILD_PYTHON_VERSION; this crate
    // validates the build environment so the user catches a misconfigured
    // PYO3_PYTHON or R2X_PYTHON_VERSION early, before the actual build.
    if let Err(error) = build_python_version::detect_build_python_version() {
        println!("cargo:error={error}");
        std::process::exit(1);
    }
}
