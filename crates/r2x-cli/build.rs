fn main() {
    // Rebuild if the Python target changes
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");

    let Ok(target) = std::env::var("TARGET") else {
        return;
    };

    // Add rpath for finding bundled libraries next to the executable
    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/..");
    } else if target.contains("linux") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/..");

        // Embed the actual Python library directory so the binary can find
        // libpython at runtime without fix_python_dylib.sh or LD_LIBRARY_PATH.
        // Release builds override this with patchelf via fix_python_dylib.sh.
        if let Some(libdir) = find_python_libdir() {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", libdir);
        }
    }
}

/// Ask the Python interpreter where libpython lives.
fn find_python_libdir() -> Option<String> {
    let python = std::env::var("PYO3_PYTHON").ok()?;
    let output = std::process::Command::new(&python)
        .args([
            "-c",
            "import sysconfig; print(sysconfig.get_config_var('LIBDIR'))",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let libdir = String::from_utf8(output.stdout).ok()?;
    let libdir = libdir.trim();
    if libdir.is_empty() {
        return None;
    }
    Some(libdir.to_string())
}
