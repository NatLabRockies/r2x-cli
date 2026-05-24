#[path = "../../scripts/build_python_version.rs"]
mod build_python_version;

fn main() {
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");
    println!("cargo:rerun-if-env-changed=R2X_PYTHON_VERSION");
    println!("cargo:rerun-if-changed=../../scripts/build_python_version.rs");

    match build_python_version::detect_build_python_version() {
        Ok(Some(version)) => println!("cargo:rustc-env=R2X_BUILD_PYTHON_VERSION={version}"),
        Ok(None) => {}
        Err(error) => {
            println!("cargo:error={error}");
            std::process::exit(1);
        }
    }
}
