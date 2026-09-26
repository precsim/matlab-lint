mod cli;
mod config;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use cli::{Cli, Command};
use config::{Config, LineEndings};
use mstyle::formatter::{FormatterOptions, LineEnding, format_source};
use mstyle::source::SourceFile;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn main() -> ExitCode {
    let cli = Cli::parse();

    let config = match cli.config.as_deref() {
        Some(path) => Config::load(path),
        None => Config::load_if_exists(Path::new("mstyle.toml")),
    };

    let config = match config {
        Ok(config) => config,
        Err(error) => {
            eprintln!("mstyle: configuration error: {error}");
            return ExitCode::from(2);
        }
    };

    run_command(cli.command, &config)
}

fn run_command(command: Command, config: &Config) -> ExitCode {
    match command {
        Command::Check { paths, format } => {
            let _ = (paths, format);
            not_implemented("check")
        }
        Command::Fmt { paths, check } => run_fmt(paths, check, config),
        Command::Lint { paths, format } => {
            let _ = (paths, format);
            not_implemented("lint")
        }
    }
}

fn run_fmt(paths: Vec<PathBuf>, check: bool, config: &Config) -> ExitCode {
    if paths.is_empty() {
        eprintln!("mstyle: fmt requires at least one explicit .m file");
        return ExitCode::from(2);
    }

    let options = FormatterOptions {
        indent_width: config.format.indent_width,
        line_ending: match config.format.line_endings {
            LineEndings::Lf => LineEnding::Lf,
            LineEndings::Crlf => LineEnding::Crlf,
        },
    };

    let mut source_diagnostics = false;

    for path in paths {
        if path.extension().and_then(|extension| extension.to_str()) != Some("m") {
            eprintln!("mstyle: {}: expected a .m file", path.display());
            return ExitCode::from(2);
        }
        if !path.is_file() {
            eprintln!(
                "mstyle: {}: explicit file paths are required until repository discovery is implemented",
                path.display()
            );
            return ExitCode::from(2);
        }

        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", path.display());
                return ExitCode::from(2);
            }
        };
        let source = SourceFile::new(&path, text);
        let outcome = match format_source(&source, options) {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", path.display());
                return ExitCode::from(2);
            }
        };

        for diagnostic in outcome.parse_diagnostics() {
            eprintln!(
                "{}:{}:{} PARSE {}",
                path.display(),
                diagnostic.start.line,
                diagnostic.start.column,
                diagnostic.message
            );
            source_diagnostics = true;
        }

        if outcome.changed() {
            if check {
                eprintln!("{}: needs formatting", path.display());
                source_diagnostics = true;
            } else if let Err(error) = write_transactional(&path, outcome.output()) {
                eprintln!("mstyle: {}: {error}", path.display());
                return ExitCode::from(2);
            }
        }
    }

    if source_diagnostics {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn write_transactional(path: &Path, contents: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let permissions = fs::metadata(path)?.permissions();

    loop {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".{}.mstyle-tmp-{}-{sequence}",
            file_name.to_string_lossy(),
            std::process::id()
        ));

        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };

        let result = (|| -> io::Result<()> {
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::set_permissions(&temporary, permissions.clone())?;
            fs::rename(&temporary, path)?;
            Ok(())
        })();

        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return result;
    }
}

fn not_implemented(command: &str) -> ExitCode {
    eprintln!("mstyle: '{command}' is not implemented in this phase");
    ExitCode::from(2)
}
