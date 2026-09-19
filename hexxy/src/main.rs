mod defs;
mod generate;
mod hex;
mod map;
mod random;
mod render;

use std::{
    fmt::Display,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(version, about = "Hex map tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

// Parsed once at startup, so the size of the largest variant costs nothing worth boxing it for.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand, Debug)]
enum Command {
    /// Expand an atlas map into sub-hexes, following Welsh Piper's Hex-based Campaign Design
    #[command(long_about = generate::LONG_ABOUT)]
    Generate(generate::Args),

    /// Render a staggered text grid of terrain as a layered hex map SVG for Foundry VTT
    #[command(long_about = render::LONG_ABOUT, allow_negative_numbers = true)]
    Render(render::Args),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Generate(args) => generate::run(args),
        Command::Render(args) => render::run(args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn warn(message: impl Display) {
    eprintln!("warning: {message}");
}

/// `given`, or else the first of `defaults` in the current directory that passes `exists`.
fn found(given: Option<&Path>, defaults: &[&str], exists: fn(&Path) -> bool) -> Option<PathBuf> {
    match given {
        Some(path) => Some(path.to_path_buf()),
        None => defaults.iter().map(PathBuf::from).find(|path| exists(path)),
    }
}
