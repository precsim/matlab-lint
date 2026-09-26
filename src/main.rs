mod cli;
mod config;

use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use cli::{Cli, Command};
use config::Config;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let config = match cli.config.as_deref() {
        Some(path) => Config::load(path),
        None => Config::load_if_exists(Path::new("mstyle.toml")),
    };

    if let Err(error) = config {
        eprintln!("mstyle: configuration error: {error}");
        return ExitCode::from(2);
    }

    run_command(cli.command)
}

fn run_command(command: Command) -> ExitCode {
    match command {
        Command::Check { paths, format } => {
            let _ = (paths, format);
            not_implemented("check")
        }
        Command::Fmt { paths, check } => {
            let _ = (paths, check);
            not_implemented("fmt")
        }
        Command::Lint { paths, format } => {
            let _ = (paths, format);
            not_implemented("lint")
        }
    }
}

fn not_implemented(command: &str) -> ExitCode {
    eprintln!("mstyle: '{command}' is not implemented in Phase 0");
    ExitCode::from(2)
}
