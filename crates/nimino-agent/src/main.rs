fn main() {
    if let Err(e) = nimino_agent::run() {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
