use std::env;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let target = env::var("TARGET").expect("TARGET not set");
    let shim_dir = env::var("R2X_PYTHON_SHIM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("../../python-shim").join(&target));

    println!("cargo:rerun-if-env-changed=R2X_PYTHON_SHIM_DIR");
    println!("cargo:rerun-if-env-changed=PY_VERSION");
    let shim_script = manifest_dir.join("../../scripts/prepare_python_shim.sh");
    if shim_script.exists() {
        println!("cargo:rerun-if-changed={}", shim_script.display());
    }

    if !shim_dir.exists() {
        return;
    }

    if let Some(lib_file) = find_python_lib(&shim_dir) {
        println!("cargo:rustc-link-search=native={}", shim_dir.display());
        println!("cargo:rerun-if-changed={}", lib_file.display());
        copy_to_profile_dir(&lib_file);
        add_rpath(&target);
    }
}

fn find_python_lib(dir: &Path) -> Option<PathBuf> {
    if let Ok(entries) = dir.read_dir() {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if name.starts_with("libpython")
                    && (name.ends_with(".dylib") || name.ends_with(".so"))
                {
                    return Some(path);
                }
                if name.starts_with("python") && name.ends_with(".dll") {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn add_rpath(target: &str) {
    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/..");
    } else if target.contains("linux") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/..");
    }
}

fn copy_to_profile_dir(lib_file: &Path) {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_path = PathBuf::from(out_dir);
    let mut ancestors = out_path.as_path().ancestors();
    let profile_dir = ancestors
        .nth(3)
        .expect("failed to resolve profile dir (target/<profile>)");
    let dest = Path::new(profile_dir).join(
        lib_file
            .file_name()
            .expect("library file should have a name"),
    );
    std::fs::create_dir_all(profile_dir).expect("failed to create profile dir");
    if let Err(err) = std::fs::copy(lib_file, &dest) {
        panic!("failed to copy python shim to {}: {}", dest.display(), err);
    }
}
