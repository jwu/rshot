#[cfg(target_os = "macos")]
mod commands;
#[cfg(target_os = "macos")]
mod error;
#[cfg(target_os = "macos")]
mod output;

#[cfg(target_os = "macos")]
use {
    clap::{CommandFactory, Parser, Subcommand, error::ErrorKind},
    commands::{annotate, capture, elements, grid, locate, ocr, windows, zoom},
    error::{CliError, ExitStatus},
    output::{CommandOutput, emit},
    std::{ffi::OsString, io, process::ExitCode},
};

#[cfg(target_os = "macos")]
#[derive(Parser)]
#[command(version, about = "Command-line interface for Rshot")]
struct Args {
    #[arg(
        long,
        global = true,
        help = "Write a single JSON response to standard output"
    )]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[cfg(target_os = "macos")]
#[derive(Subcommand)]
enum Command {
    /// Capture a display, window, region, or accessibility element.
    Capture(capture::Args),
    /// List windows.
    Windows(windows::Args),
    /// Locate accessibility elements.
    Elements(elements::Args),
    /// Annotate an image.
    Annotate(annotate::Args),
    /// Draw a grid over an image.
    Grid(grid::Args),
    /// Enlarge a region of an image.
    Zoom(zoom::Args),
    /// Recognize text and report line coordinates in an image.
    Ocr(ocr::Args),
    /// Find the pixel coordinates of text in an image.
    Locate(locate::Args),
}

#[cfg(target_os = "macos")]
fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    let (command, json, result) = match Args::try_parse_from(&args) {
        Ok(args) => {
            let (command, result) = match args.command {
                Command::Capture(args) => ("capture", capture::run(args)),
                Command::Windows(args) => ("windows", windows::run(args)),
                Command::Elements(args) => ("elements", elements::run(args)),
                Command::Annotate(args) => ("annotate", annotate::run(args)),
                Command::Grid(args) => ("grid", grid::run(args)),
                Command::Zoom(args) => ("zoom", zoom::run(args)),
                Command::Ocr(args) => ("ocr", ocr::run(args)),
                Command::Locate(args) => ("locate", locate::run(args)),
            };
            (command.to_owned(), args.json, result)
        }
        Err(error) => {
            let json = args
                .iter()
                .skip(1)
                .take_while(|arg| *arg != "--")
                .any(|arg| arg == "--json");
            if !json {
                error.exit();
            }
            (invoked_command(&args), true, parse_response(error))
        }
    };

    match emit(
        &command,
        json,
        result,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Could not write the CLI response ({error}).");
            ExitCode::from(ExitStatus::Io as u8)
        }
    }
}

#[cfg(target_os = "macos")]
fn invoked_command(args: &[OsString]) -> String {
    let mut args = args.iter().skip(1).filter(|arg| *arg != "--json");
    let name = match args.next() {
        Some(arg) if arg == "help" => args.next(),
        arg => arg,
    };
    name.and_then(|arg| arg.to_str())
        .and_then(|name| {
            Args::command()
                .find_subcommand(name)
                .map(|command| command.get_name().to_owned())
        })
        .unwrap_or_else(|| env!("CARGO_PKG_NAME").to_owned())
}

#[cfg(target_os = "macos")]
fn parse_response(error: clap::Error) -> Result<CommandOutput, CliError> {
    let (field, text) = match error.kind() {
        ErrorKind::DisplayHelp => ("help", error.to_string()),
        ErrorKind::DisplayVersion => ("version", env!("CARGO_PKG_VERSION").to_owned()),
        _ => {
            return Err(CliError::new(
                ExitStatus::Usage,
                "invalid_usage",
                error.to_string().trim_end(),
            ));
        }
    };
    let fields = serde_json::Map::from_iter([(field.to_owned(), serde_json::Value::String(text))]);
    Ok(CommandOutput::new(fields, ""))
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Rshot currently supports macOS 14 and later.");
    std::process::exit(1);
}
