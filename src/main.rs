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
        Command::Check { paths, format } => run_check(paths, format, config),
        Command::Fmt { paths, check } => run_fmt(paths, check, config),
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

fn validate_explicit_files(paths: &[PathBuf], command: &str) -> Result<(), ExitCode> {
    if paths.is_empty() {
        eprintln!("mstyle: {command} requires at least one explicit .m file");
        return Err(ExitCode::from(2));
    }

    for path in paths {
        if path.extension().and_then(|extension| extension.to_str()) != Some("m") {
            eprintln!("mstyle: {}: expected a .m file", path.display());
            return Err(ExitCode::from(2));
        }
        if !path.is_file() {
            eprintln!(
                "mstyle: {}: explicit file paths are required until repository discovery is implemented",
                path.display()
            );
            return Err(ExitCode::from(2));
        }
    }

    Ok(())
}

fn read_source(path: &Path) -> Result<SourceFile, ExitCode> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(SourceFile::new(path, text)),
        Err(error) => {
            eprintln!("mstyle: {}: {error}", path.display());
            Err(ExitCode::from(2))
        }
    }
}

fn run_check(paths: Vec<PathBuf>, output_format: OutputFormat, config: &Config) -> ExitCode {
    if let Err(code) = validate_explicit_files(&paths, "check") {
        return code;
    }

    let mut diagnostics = Vec::new();
    let mut parser = match MatlabParser::new() {
        Ok(parser) => parser,
        Err(error) => {
            eprintln!("mstyle: {error}");
            return ExitCode::from(2);
        }
    };

    for path in paths {
        let source = match read_source(&path) {
            Ok(source) => source,
            Err(code) => return code,
        };
        let parsed = match parser.parse(&source) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", path.display());
                return ExitCode::from(2);
            }
        };

        diagnostics.extend(
            parsed
                .diagnostics()
                .iter()
                .map(|diagnostic| Diagnostic::from_parse(&path, diagnostic)),
        );

        let formatted = match format_source_with_parse(&source, formatter_options(config), &parsed)
        {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", path.display());
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
                    eprintln!("mstyle: {}: {error}", path.display());
                    return ExitCode::from(2);
                }
            };
            diagnostics.push(diagnostic);
        }

        match lint_source(&source, &parsed, lint_options(config)) {
            Ok(mut lint_diagnostics) => diagnostics.append(&mut lint_diagnostics),
            Err(error) => {
                eprintln!("mstyle: {}: {error}", path.display());
                return ExitCode::from(2);
            }
        }
    }

    emit_diagnostics(&mut diagnostics, output_format)
}

fn run_lint(paths: Vec<PathBuf>, output_format: OutputFormat, config: &Config) -> ExitCode {
    if let Err(code) = validate_explicit_files(&paths, "lint") {
        return code;
    }

    let mut diagnostics = Vec::new();
    let mut parser = match MatlabParser::new() {
        Ok(parser) => parser,
        Err(error) => {
            eprintln!("mstyle: {error}");
            return ExitCode::from(2);
        }
    };

    for path in paths {
        let source = match read_source(&path) {
            Ok(source) => source,
            Err(code) => return code,
        };
        let parsed = match parser.parse(&source) {
            Ok(parsed) => parsed,
            Err(error) => {
                eprintln!("mstyle: {}: {error}", path.display());
                return ExitCode::from(2);
            }
        };

        diagnostics.extend(
            parsed
                .diagnostics()
                .iter()
                .map(|diagnostic| Diagnostic::from_parse(&path, diagnostic)),
        );

        match lint_source(&source, &parsed, lint_options(config)) {
            Ok(mut lint_diagnostics) => diagnostics.append(&mut lint_diagnostics),
            Err(error) => {
                eprintln!("mstyle: {}: {error}", path.display());
                return ExitCode::from(2);
            }
        }
    }

    emit_diagnostics(&mut diagnostics, output_format)
}

fn emit_diagnostics(diagnostics: &mut Vec<Diagnostic>, output_format: OutputFormat) -> ExitCode {
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

fn run_fmt(paths: Vec<PathBuf>, check: bool, config: &Config) -> ExitCode {
    if let Err(code) = validate_explicit_files(&paths, "fmt") {
        return code;
    }

    let options = formatter_options(config);
    let mut source_diagnostics = false;

    for path in paths {
        let source = match read_source(&path) {
            Ok(source) => source,
            Err(code) => return code,
        };
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
