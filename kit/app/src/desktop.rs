#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
fn main() -> eframe::Result {
    if std::env::args_os().any(|a| a == "--snapshot") {
        if let Err(error) = vector_magic_rebuild::preview_cli::run() {
            eprintln!("Error: {error}");
            std::process::exit(2);
        }
        return Ok(());
    }
    vector_magic_rebuild::desktop_ui::run()
}
