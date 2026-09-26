mod cli;

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use cli::{Cli, Command, OutputFormat};
use mstyle::config::{Config, LineEndings};
use mstyle::diagnostic::{Diagnostic, DiagnosticReport, sort_diagnostics};
use mstyle::discovery::{FileTarget, discover_diff, discover_paths};
use mstyle::formatter::{FormatterOptions, LineEnding, format_source, format_source_with_parse};
use mstyle::lint::{LintOptions, lint_source};
use mstyle::parser::MatlabParser;
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
        Command::Check {
            paths,
            format,
            diff,
        } => run_check(paths, diff.as_deref(), format, config),
        Command::Fmt { paths, check, diff } => run_fmt(paths, check, diff.as_deref(), config),
        Command::Lint { paths, format } => run_lint(paths, format, config),
    }
}

fn formatter_options(config: &Config) -> FormatterOptions {
    FormatterOptions {
        indent_width: config.format.indent_width,
        line_ending: match config.format.line_endings {
            LineEndings::Lf => LineEnding::Lf,
            LineEndings::Crlf => LineEnding::Crlf,
        },
    }
}

fn lint_options(config: &Config) -> LintOptions {
    LintOptions {
        line_length: config.lint.line_length,
        line_length_severity: config.severity("L001"),
        multiple_statements_severity: config.severity("L002"),
    }
}

fn resolve_targets(
    paths: &[PathBuf],
    diff: Option<&str>,
    config: &Config,
) -> Result<Vec<FileTarget>, ExitCode> {
    let result = match diff {
        Some(base) => discover_diff(base, &config.exclude.paths),
        None => discover_paths(paths, &config.exclude.paths),
    };

    result.map_err(|error| {
        eprintln!("mstyle: discovery error: {error}");
        ExitCode::from(2)
    })
}

fn read_source(target: &FileTarget) -> Result<SourceFile, ExitCode> {
    match fs::read_to_string(&target.io_path) {
        Ok(text) => Ok(SourceFile::new(&target.display_path, text)),
        Err(error) => {
            eprintln!("mstyle: {}: {error}", target.display_path.display());
            Err(ExitCode::from(2))
        }
    }
}

fn run_check(
    paths: Vec<PathBuf>,
    diff: Option<&str>,
    output_format: OutputFormat,
    config: &Config,
) -> ExitCode {
    let targets = match resolve_targets(&paths, diff, config) {
        Ok(targets) => targets,
        Err(code) => return code,
    };

    let mut diagnostics = Vec::new();
    let mut parser = match MatlabParser::new() {
        Ok(parser) => parser,
        Err(error) => {
            eprintln!("mstyle: {error}");
            return ExitCode::from(2);
        }
    };

    for target in targets {
        let source = match read_source(&target) {
            Ok(source) => source,
            Err(code) => return code,
        };
        let parsed = match parser.parse(&source) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", source.path().display());
                return ExitCode::from(2);
            }
        };

        diagnostics.extend(
            parsed
                .diagnostics()
                .iter()
                .map(|diagnostic| Diagnostic::from_parse(source.path(), diagnostic)),
        );

        let formatted = match format_source_with_parse(&source, formatter_options(config), &parsed)
        {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", source.path().display());
                return ExitCode::from(2);
            }
        };

        for edit in formatted.edits() {
            let diagnostic = match Diagnostic::from_formatter_edit(
                &source,
                edit,
                config.severity(&edit.rule_id),
            ) {
                Ok(diagnostic) => diagnostic,
                Err(error) => {
                    eprintln!("mstyle: {}: {error}", source.path().display());
                    return ExitCode::from(2);
                }
            };
            diagnostics.push(diagnostic);
        }

        match lint_source(&source, &parsed, lint_options(config)) {
            Ok(mut lint_diagnostics) => diagnostics.append(&mut lint_diagnostics),
            Err(error) => {
                eprintln!("mstyle: {}: {error}", source.path().display());
                return ExitCode::from(2);
            }
        }
    }

    emit_diagnostics(&mut diagnostics, output_format)
}

fn run_lint(paths: Vec<PathBuf>, output_format: OutputFormat, config: &Config) -> ExitCode {
    let targets = match resolve_targets(&paths, None, config) {
        Ok(targets) => targets,
        Err(code) => return code,
    };

    let mut diagnostics = Vec::new();
    let mut parser = match MatlabParser::new() {
        Ok(parser) => parser,
        Err(error) => {
            eprintln!("mstyle: {error}");
            return ExitCode::from(2);
        }
    };

    for target in targets {
        let source = match read_source(&target) {
            Ok(source) => source,
            Err(code) => return code,
        };
        let parsed = match parser.parse(&source) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", source.path().display());
                return ExitCode::from(2);
            }
        };

        diagnostics.extend(
            parsed
                .diagnostics()
                .iter()
                .map(|diagnostic| Diagnostic::from_parse(source.path(), diagnostic)),
        );

        match lint_source(&source, &parsed, lint_options(config)) {
            Ok(mut lint_diagnostics) => diagnostics.append(&mut lint_diagnostics),
            Err(error) => {
                eprintln!("mstyle: {}: {error}", source.path().display());
                return ExitCode::from(2);
            }
        }
    }

    emit_diagnostics(&mut diagnostics, output_format)
}

fn emit_diagnostics(diagnostics: &mut [Diagnostic], output_format: OutputFormat) -> ExitCode {
    sort_diagnostics(diagnostics);

    match output_format {
        OutputFormat::Text => {
            for diagnostic in diagnostics.iter() {
                eprintln!(
                    "{}:{}:{} {} {}",
                    diagnostic.file,
                    diagnostic.start.line,
                    diagnostic.start.column,
                    diagnostic.rule,
                    diagnostic.message
                );
            }
        }
        OutputFormat::Json => {
            let report = DiagnosticReport::new(diagnostics);
            match serde_json::to_string(&report) {
                Ok(json) => println!("{json}"),
                Err(error) => {
                    eprintln!("mstyle: failed to serialize diagnostics: {error}");
                    return ExitCode::from(2);
                }
            }
        }
    }

    if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn run_fmt(paths: Vec<PathBuf>, check: bool, diff: Option<&str>, config: &Config) -> ExitCode {
    let targets = match resolve_targets(&paths, diff, config) {
        Ok(targets) => targets,
        Err(code) => return code,
    };

    let options = formatter_options(config);
    let mut source_diagnostics = false;

    for target in targets {
        let source = match read_source(&target) {
            Ok(source) => source,
            Err(code) => return code,
        };
        let outcome = match format_source(&source, options) {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", source.path().display());
                return ExitCode::from(2);
            }
        };

        for diagnostic in outcome.parse_diagnostics() {
            eprintln!(
                "{}:{}:{} PARSE {}",
                source.path().display(),
                diagnostic.start.line,
                diagnostic.start.column,
                diagnostic.message
            );
            source_diagnostics = true;
        }

        if outcome.changed() {
            if check {
                eprintln!("{}: needs formatting", source.path().display());
                source_diagnostics = true;
            } else if let Err(error) = write_transactional(&target.io_path, outcome.output()) {
                eprintln!("mstyle: {}: {error}", source.path().display());
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
