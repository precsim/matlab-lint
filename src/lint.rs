use std::collections::BTreeMap;

use tree_sitter::Node;

use crate::diagnostic::{Diagnostic, Severity};
use crate::parser::ParseResult;
use crate::source::{ByteRange, PositionError, SourceFile};

#[derive(Debug, Clone, Copy)]
pub struct LintOptions {
    pub line_length: usize,
    pub line_length_severity: Severity,
    pub multiple_statements_severity: Severity,
}

pub fn lint_source(
    source: &SourceFile,
    parsed: &ParseResult,
    options: LintOptions,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut diagnostics = line_length_diagnostics(source, options)?;

    if parsed.structural_formatting_allowed() {
        diagnostics.extend(multiple_statement_diagnostics(
            source,
            parsed.tree().root_node(),
            options.multiple_statements_severity,
        )?);
    }

    Ok(diagnostics)
}

fn line_length_diagnostics(
    source: &SourceFile,
    options: LintOptions,
) -> Result<Vec<Diagnostic>, PositionError> {
    if options.line_length == 0 {
        return Ok(Vec::new());
    }

    let mut diagnostics = Vec::new();
    let text = source.text();
    let bytes = text.as_bytes();
    let mut line_start = 0;

    while line_start <= bytes.len() {
        let newline = bytes[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| line_start + offset)
            .unwrap_or(bytes.len());
        let mut content_end = newline;
        if content_end > line_start && bytes[content_end - 1] == b'\r' {
            content_end -= 1;
        }

        let content = &text[line_start..content_end];
        if content.chars().count() > options.line_length {
            let overflow_start = byte_after_scalars(content, options.line_length) + line_start;
            diagnostics.push(Diagnostic {
                rule: "L001".to_owned(),
                severity: options.line_length_severity,
                file: source.path().to_string_lossy().into_owned(),
                start: source.position(overflow_start)?,
                end: source.position(content_end)?,
                start_byte: overflow_start,
                end_byte: content_end,
                message: format!(
                    "line too long ({} > {})",
                    content.chars().count(),
                    options.line_length
                ),
                fixable: false,
            });
        }

        if newline == bytes.len() {
            break;
        }
        line_start = newline + 1;
    }

    Ok(diagnostics)
}

fn byte_after_scalars(text: &str, scalar_count: usize) -> usize {
    text.char_indices()
        .nth(scalar_count)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

fn multiple_statement_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut second_statement_by_row = BTreeMap::<usize, ByteRange>::new();
    collect_multiple_statements(root, &mut second_statement_by_row);

    second_statement_by_row
        .into_values()
        .map(|range| {
            Ok(Diagnostic {
                rule: "L002".to_owned(),
                severity,
                file: source.path().to_string_lossy().into_owned(),
                start: source.position(range.start)?,
                end: source.position(range.end)?,
                start_byte: range.start,
                end_byte: range.end,
                message: "multiple statements on one line".to_owned(),
                fixable: false,
            })
        })
        .collect()
}

fn collect_multiple_statements(node: Node<'_>, diagnostics: &mut BTreeMap<usize, ByteRange>) {
    if node.kind() == "block" {
        let mut first_by_row = BTreeMap::<usize, usize>::new();
        let mut cursor = node.walk();

        for child in node.named_children(&mut cursor) {
            if matches!(child.kind(), "comment" | "line_continuation") {
                continue;
            }

            let row = child.start_position().row;
            if first_by_row.insert(row, child.start_byte()).is_some() {
                diagnostics.entry(row).or_insert_with(|| {
                    ByteRange::new(child.start_byte(), child.end_byte())
                });
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_multiple_statements(child, diagnostics);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::MatlabParser;

    fn lint(text: &str, line_length: usize) -> Vec<Diagnostic> {
        let source = SourceFile::new("lint.m", text);
        let mut parser = MatlabParser::new().expect("grammar");
        let parsed = parser.parse(&source).expect("parse");
        lint_source(
            &source,
            &parsed,
            LintOptions {
                line_length,
                line_length_severity: Severity::Warning,
                multiple_statements_severity: Severity::Warning,
            },
        )
        .expect("lint")
    }

    #[test]
    fn line_length_counts_unicode_scalars() {
        let diagnostics = lint("éééé\n", 3);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.rule == "L001")
            .expect("L001");

        assert_eq!(diagnostic.start.column, 4);
        assert_eq!(diagnostic.start_byte, 6);
    }

    #[test]
    fn reports_multiple_statements_on_one_line() {
        let diagnostics = lint("x = 1; y = 2;\n", 120);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.rule == "L002")
        );
    }

    #[test]
    fn matrix_row_separators_are_not_multiple_statements() {
        let diagnostics = lint("A = [1,2;3,4];\n", 120);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule != "L002")
        );
    }

    #[test]
    fn malformed_source_skips_structural_lint() {
        let diagnostics = lint("x = 1 + ; y = 2;\n", 120);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule != "L002")
        );
    }
}
