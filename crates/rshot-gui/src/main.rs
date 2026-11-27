#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod ui;

#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    #[cfg(target_os = "macos")]
    ui::run();

    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("Rshot currently supports macOS 14 and later.");
        std::process::exit(1);
    }
}
