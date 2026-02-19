fn main() {
    println!("cargo:rerun-if-env-changed=PYO3_PYTHON");

    let Ok(target) = std::env::var("TARGET") else {
        return;
    };

    if target.contains("apple-darwin") {
        // Relative rpaths for portable installation
        add_rpath("@executable_path");
        add_rpath("@executable_path/../lib");

        // Homebrew locations
        add_rpath("/opt/homebrew/lib");
        add_rpath("/usr/local/lib");

        // Python framework
        add_rpath("/Library/Frameworks/Python.framework/Versions/Current/lib");

        // Build-time Python LIBDIR (useful for local dev, harmless in release)
        if let Some(libdir) = find_python_libdir() {
            add_rpath(&libdir);
        }
    } else if target.contains("linux") {
        // Relative rpaths for portable installation
        add_rpath("$ORIGIN");
        add_rpath("$ORIGIN/../lib");

        // Standard system library paths
        add_rpath("/usr/lib");
        add_rpath("/usr/lib64");
        add_rpath("/usr/local/lib");

        // Build-time Python LIBDIR
        if let Some(libdir) = find_python_libdir() {
            add_rpath(&libdir);
        }
    }
}

fn add_rpath(path: &str) {
    println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
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
