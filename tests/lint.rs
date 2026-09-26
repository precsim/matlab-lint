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
        },
    )
    .expect("lint");

    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].rule, "L001");
    assert_eq!(diagnostics[0].severity, Severity::Warning);
    assert_eq!(diagnostics[1].rule, "L002");
    assert_eq!(diagnostics[1].severity, Severity::Error);
}
