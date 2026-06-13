fn main() {
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");
    println!("cargo:rerun-if-env-changed=R2X_PYTHON_VERSION");

    match r2x_build_support::detect_build_python_version() {
        Ok(Some(version)) => println!("cargo:rustc-env=R2X_BUILD_PYTHON_VERSION={version}"),
        Ok(None) => {}
        Err(error) => {
            println!("cargo:error={error}");
            std::process::exit(1);
        }
    }
}
