use mstyle::diagnostic::Severity;
use mstyle::lint::{LintOptions, lint_source};
use mstyle::parser::MatlabParser;
use mstyle::source::SourceFile;

#[test]
fn lint_reports_line_length_and_multiple_statements_deterministically() {
    let source = SourceFile::new("lint.m", "x = 1; y = 2; % abcdef\n");
    let mut parser = MatlabParser::new().expect("grammar");
    let parsed = parser.parse(&source).expect("parse");

    let diagnostics = lint_source(
        &source,
        &parsed,
        LintOptions {
            line_length: 10,
            line_length_severity: Severity::Warning,
            multiple_statements_severity: Severity::Error,
            global_severity: Severity::Warning,
            eval_severity: Severity::Warning,
            wildcard_import_severity: Severity::Warning,
            file_name_mismatch_severity: Severity::Warning,
            duplicate_local_function_severity: Severity::Warning,
            duplicate_case_severity: Severity::Warning,
            unreachable_statement_severity: Severity::Warning,
            missing_semicolon_severity: None,
        },
    )
    .expect("lint");

    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].rule, "L001");
    assert_eq!(diagnostics[0].severity, Severity::Warning);
    assert_eq!(diagnostics[1].rule, "L002");
    assert_eq!(diagnostics[1].severity, Severity::Error);
}
