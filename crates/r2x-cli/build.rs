fn main() {
    let target = std::env::var("TARGET").expect("TARGET not set");

    // Add rpath for finding bundled libraries next to the executable
    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/..");
    } else if target.contains("linux") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/..");
    }
}
