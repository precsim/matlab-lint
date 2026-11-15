mod cli;

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use cli::{Cli, Command, OutputFormat, VcsOverride};
use mstyle::config::{Config, DiffVcs, LineEndings};
use mstyle::diagnostic::{Diagnostic, DiagnosticReport, sort_diagnostics};
use mstyle::diff::{DiffAnalysis, DiffDecision, analyze_diff};
use mstyle::discovery::{
    FileTarget, discover_changed, discover_diff, discover_paths, discover_working_tree_changes,
};
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
            changed,
        } => run_check(paths, diff.as_deref(), changed, format, config),
        Command::Fmt {
            paths,
            check,
            diff,
            stdin,
        } => {
            if stdin {
                run_fmt_stdin(config)
            } else {
                run_fmt(paths, check, diff.as_deref(), config)
            }
        }
        Command::Lint { paths, format } => run_lint(paths, format, config),
        Command::DiffCheck { format, vcs } => run_diff_check(format, vcs, config),
    }
}

fn run_diff_check(
    output_format: OutputFormat,
    vcs_override: Option<VcsOverride>,
    config: &Config,
) -> ExitCode {
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(error) => {
            eprintln!("mstyle: diff discovery error: {error}");
            return ExitCode::from(2);
        }
    };
    let vcs = match vcs_override {
        Some(VcsOverride::Git) => DiffVcs::Git,
        Some(VcsOverride::Fossil) => DiffVcs::Fossil,
        None => config.diff.vcs,
    };
    let workspace = match discover_working_tree_changes(&cwd, vcs) {
        Ok(workspace) => workspace,
        Err(error) => {
            eprintln!("mstyle: diff discovery error: {error}");
            return ExitCode::from(2);
        }
    };
    let analysis = match analyze_diff(&workspace, &config.diff) {
        Ok(analysis) => analysis,
        Err(error) => {
            eprintln!("mstyle: diff analysis error: {error}");
            return ExitCode::from(2);
        }
    };

    if let Err(code) = emit_diff_analysis(&analysis, output_format) {
        return code;
    }

    if analysis.source_diagnostics
        || (!analysis.advisory && analysis.decision != DiffDecision::Accept)
    {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn emit_diff_analysis(
    analysis: &DiffAnalysis,
    output_format: OutputFormat,
) -> Result<(), ExitCode> {
    match output_format {
        OutputFormat::Json => {
            let json = serde_json::to_string(analysis).map_err(|error| {
                eprintln!("mstyle: failed to serialize diff analysis: {error}");
                ExitCode::from(2)
            })?;
            println!("{json}");
        }
        OutputFormat::Text => {
            println!("decision: {}", analysis.decision);
            println!("vcs: {}", analysis.vcs);
            println!("advisory: {}", analysis.advisory);
            emit_diff_metrics("implementation", analysis.metrics);
            emit_diff_metrics("tests", analysis.test_metrics);
            for profile in &analysis.profiles {
                emit_diff_metrics(&format!("profile {}", profile.name), profile.metrics);
            }
            for trigger in &analysis.triggers {
                match (&trigger.metric, trigger.value, trigger.threshold) {
                    (Some(metric), Some(value), Some(threshold)) => println!(
                        "trigger: {} {} {} value={} threshold={}",
                        trigger.severity, trigger.id, metric, value, threshold
                    ),
                    _ => println!("trigger: {} {}", trigger.severity, trigger.id),
                }
            }
            for path in &analysis.generated_files {
                println!("generated: {path}");
            }
            for file in &analysis.files {
                let path = file
                    .new_path
                    .as_deref()
                    .or(file.old_path.as_deref())
                    .unwrap_or("<unknown>");
                println!(
                    "file: {path} class={} parse_confidence={} embedded_test_regions_touched={}",
                    file.classification, file.parse_confidence, file.embedded_test_regions_touched
                );
                for function in &file.functions {
                    println!("function: {} {}", function.change, function.name);
                }
                for fact in &file.facts {
                    print!(
                        "fact: {} change={} side={} path={} range={}:{}-{}:{} bytes={}..{}",
                        fact.kind,
                        fact.change,
                        fact.side,
                        fact.path,
                        fact.range.start.line,
                        fact.range.start.column,
                        fact.range.end.line,
                        fact.range.end.column,
                        fact.range.start_byte,
                        fact.range.end_byte
                    );
                    if let Some(function) = &fact.enclosing_function {
                        print!(" function={function}");
                    }
                    emit_fact_values("identifiers", &fact.identifiers);
                    emit_fact_values("targets", &fact.call_or_index_targets);
                    emit_fact_values("field_paths", &fact.field_paths);
                    emit_fact_values("operators", &fact.operators);
                    emit_fact_values("literals", &fact.literals);
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn emit_fact_values(label: &str, values: &[String]) {
    if !values.is_empty() {
        print!(" {label}=[{}]", values.join(","));
    }
}

fn emit_diff_metrics(scope: &str, metrics: mstyle::diff::DiffMetrics) {
    println!("{scope}:");
    println!(
        "  executable lines added: {}",
        metrics.added_executable_lines
    );
    println!(
        "  executable lines deleted: {}",
        metrics.deleted_executable_lines
    );
    println!("  .m files touched: {}", metrics.files_touched);
    println!("  functions touched: {}", metrics.functions_touched);
    println!("  functions added: {}", metrics.functions_added);
    println!(
        "  control-flow complexity increase: {}",
        metrics.control_flow_increase
    );
    println!("  %% sections touched: {}", metrics.sections_touched);
    println!("  %% sections added: {}", metrics.sections_added);
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
    changed: bool,
    config: &Config,
) -> Result<Vec<FileTarget>, ExitCode> {
    let result = if changed {
        discover_changed(&config.exclude.paths)
    } else {
        match diff {
            Some(base) => discover_diff(base, &config.exclude.paths),
            None => discover_paths(paths, &config.exclude.paths),
        }
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
    changed: bool,
    output_format: OutputFormat,
    config: &Config,
) -> ExitCode {
    let targets = match resolve_targets(&paths, diff, changed, config) {
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
    let targets = match resolve_targets(&paths, None, false, config) {
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

fn run_fmt_stdin(config: &Config) -> ExitCode {
    let mut input = String::new();
    if let Err(error) = io::stdin().read_to_string(&mut input) {
        eprintln!("mstyle: failed to read stdin: {error}");
        return ExitCode::from(2);
    }

    let source = SourceFile::new("<stdin>", input);
    let outcome = match format_source(&source, formatter_options(config)) {
        Ok(outcome) => outcome,
        Err(error) => {
            eprintln!("mstyle: <stdin>: {error}");
            return ExitCode::from(2);
        }
    };

    let has_parse_diagnostics = !outcome.parse_diagnostics().is_empty();
    for diagnostic in outcome.parse_diagnostics() {
        eprintln!(
            "<stdin>:{}:{} PARSE {}",
            diagnostic.start.line, diagnostic.start.column, diagnostic.message
        );
    }

    let mut stdout = io::stdout().lock();
    if let Err(error) = stdout
        .write_all(outcome.output().as_bytes())
        .and_then(|_| stdout.flush())
    {
        eprintln!("mstyle: failed to write stdout: {error}");
        return ExitCode::from(2);
    }

    if has_parse_diagnostics {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn run_fmt(paths: Vec<PathBuf>, check: bool, diff: Option<&str>, config: &Config) -> ExitCode {
    let targets = match resolve_targets(&paths, diff, false, config) {
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
