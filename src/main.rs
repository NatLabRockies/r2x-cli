use r2x;

fn main() {
    if let Err(err) = r2x::run() {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
}
