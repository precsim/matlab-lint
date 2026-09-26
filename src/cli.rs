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
        /// Check only MATLAB files changed relative to the merge-base with BASE.
        #[arg(long, value_name = "BASE", conflicts_with_all = ["paths", "changed"])]
        diff: Option<String>,

        /// Check only staged, unstaged, and untracked MATLAB files.
        #[arg(long, conflicts_with_all = ["paths", "diff"])]
        changed: bool,

        /// Files or directories to check. Defaults to the current directory.
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

        /// Read source from stdin and write formatted source to stdout.
        #[arg(long, conflicts_with_all = ["check", "diff", "paths"])]
        stdin: bool,

        /// Format-check only MATLAB files changed relative to the merge-base with BASE.
        #[arg(
            long,
            value_name = "BASE",
            conflicts_with = "paths",
            requires = "check"
        )]
        diff: Option<String>,

        /// Files or directories to format. Defaults to the current directory.
        #[arg(value_name = "PATH")]
        paths: Vec<PathBuf>,
    },

    /// Run lint diagnostics without modifying files.
    Lint {
        /// Files or directories to lint. Defaults to the current directory.
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
            Command::Check {
                paths,
                format,
                diff,
                changed,
            } => {
                assert_eq!(paths, vec![PathBuf::from("src")]);
                assert_eq!(format, OutputFormat::Json);
                assert_eq!(diff, None);
                assert!(!changed);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parses_check_changed_command() {
        let cli = Cli::try_parse_from(["mstyle", "check", "--changed"])
            .expect("CLI should parse");

        match cli.command {
            Command::Check {
                paths,
                format: _,
                diff,
                changed,
            } => {
                assert!(paths.is_empty());
                assert!(diff.is_none());
                assert!(changed);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn rejects_check_changed_with_diff_or_paths() {
        assert!(
            Cli::try_parse_from([
                "mstyle",
                "check",
                "--changed",
                "--diff",
                "origin/main"
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["mstyle", "check", "--changed", "src"]).is_err());
    }

    #[test]
    fn parses_fmt_check_diff_command() {
        let cli = Cli::try_parse_from(["mstyle", "fmt", "--check", "--diff", "origin/main"])
            .expect("CLI should parse");

        match cli.command {
            Command::Fmt {
                paths,
                check,
                diff,
                stdin,
            } => {
                assert!(check);
                assert!(!stdin);
                assert!(paths.is_empty());
                assert_eq!(diff.as_deref(), Some("origin/main"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn rejects_mutating_fmt_diff() {
        assert!(
            Cli::try_parse_from(["mstyle", "fmt", "--diff", "origin/main"]).is_err(),
            "--diff must require --check"
        );
    }

    #[test]
    fn parses_fmt_stdin_command() {
        let cli = Cli::try_parse_from(["mstyle", "fmt", "--stdin"]).expect("CLI should parse");

        match cli.command {
            Command::Fmt {
                paths,
                check,
                diff,
                stdin,
            } => {
                assert!(stdin);
                assert!(!check);
                assert!(diff.is_none());
                assert!(paths.is_empty());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn rejects_fmt_stdin_with_paths_or_check() {
        assert!(Cli::try_parse_from(["mstyle", "fmt", "--stdin", "file.m"]).is_err());
        assert!(Cli::try_parse_from(["mstyle", "fmt", "--stdin", "--check"]).is_err());
    }
}
