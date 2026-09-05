fn main() {
    let Ok(target) = std::env::var("TARGET") else {
        return;
    };

    if target.contains("apple-darwin") {
        add_rpath("@executable_path");
        add_rpath("@executable_path/../lib");
    } else if target.contains("linux") {
        add_rpath("$ORIGIN");
        add_rpath("$ORIGIN/../lib");
    }
}

fn add_rpath(path: &str) {
    println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
}
