use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "mstyle",
    version,
    about = "Fast, conservative MATLAB/Octave formatter and linter"
)]
pub struct Cli {
    /// Read configuration from this file instead of ./mstyle.toml.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check formatting and lint diagnostics without modifying files.
    Check {
        /// Files or directories to check. Discovery is implemented in a later phase.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Diagnostic output format.
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },

    /// Format MATLAB/Octave source.
    Fmt {
        /// Report formatting changes without writing them.
        #[arg(long)]
        check: bool,

        /// Files or directories to format. Discovery is implemented in a later phase.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },

    /// Run lint diagnostics without modifying files.
    Lint {
        /// Files or directories to lint. Discovery is implemented in a later phase.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,

        /// Diagnostic output format.
        #[arg(long, value_enum, default_value = "text")]
        format: OutputFormat,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_check_json_command() {
        let cli = Cli::try_parse_from([
            "mstyle",
            "--config",
            "custom.toml",
            "check",
            "--format",
            "json",
            "src",
        ])
        .expect("CLI should parse");

        assert_eq!(cli.config, Some(PathBuf::from("custom.toml")));

        match cli.command {
            Command::Check { paths, format } => {
                assert_eq!(paths, vec![PathBuf::from("src")]);
                assert_eq!(format, OutputFormat::Json);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_fmt_check_command() {
        let cli = Cli::try_parse_from(["mstyle", "fmt", "--check", "example.m"])
            .expect("CLI should parse");

        match cli.command {
            Command::Fmt { paths, check } => {
                assert!(check);
                assert_eq!(paths, vec![PathBuf::from("example.m")]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
