#[cfg(target_os = "macos")]
use clap::Parser;

#[cfg(target_os = "macos")]
#[derive(Parser)]
#[command(version, about = "Command-line interface for Rshot")]
struct Args {}

fn main() {
    #[cfg(target_os = "macos")]
    Args::parse();

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Rshot currently supports macOS 14 and later.");
        std::process::exit(1);
    }
}
