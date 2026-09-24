fn main() {
    if let Err(error) = vector_magic_rebuild::preview_cli::run() {
        eprintln!("Error: {error}");
        std::process::exit(2);
    }
}
