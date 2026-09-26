use std::fmt;

use tree_sitter::{LanguageError, Node, Parser, Tree};

use crate::source::{ByteRange, PositionError, SourceFile, SourcePosition};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ParseIssueKind {
    Error,
    Missing { expected: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub kind: ParseIssueKind,
    pub range: ByteRange,
    pub start: SourcePosition,
    pub end: SourcePosition,
    pub message: String,
}

pub struct ParseResult {
    tree: Tree,
    diagnostics: Vec<ParseDiagnostic>,
}

impl ParseResult {
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    pub fn diagnostics(&self) -> &[ParseDiagnostic] {
        &self.diagnostics
    }

    pub fn has_errors(&self) -> bool {
        self.tree.root_node().has_error()
    }

    pub fn structural_formatting_allowed(&self) -> bool {
        !self.has_errors()
    }
}

pub struct MatlabParser {
    parser: Parser,
}

impl MatlabParser {
    pub fn new() -> Result<Self, ParserError> {
        let mut parser = Parser::new();
        let language = tree_sitter_matlab::LANGUAGE.into();
        parser
            .set_language(&language)
            .map_err(ParserError::Language)?;

        Ok(Self { parser })
    }

    pub fn parse(&mut self, source: &SourceFile) -> Result<ParseResult, ParserError> {
        let tree = self
            .parser
            .parse(source.text(), None)
            .ok_or(ParserError::Cancelled)?;

        let mut diagnostics = Vec::new();
        collect_parse_diagnostics(tree.root_node(), source, &mut diagnostics)?;
        diagnostics.sort_by(|left, right| {
            (left.range.start, left.range.end, &left.kind, &left.message).cmp(&(
                right.range.start,
                right.range.end,
                &right.kind,
                &right.message,
            ))
        });

        Ok(ParseResult { tree, diagnostics })
    }
}

fn collect_parse_diagnostics(
    node: Node<'_>,
    source: &SourceFile,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Result<(), PositionError> {
    if node.is_error() {
        diagnostics.push(make_diagnostic(
            source,
            node,
            ParseIssueKind::Error,
            "syntax error".to_owned(),
        )?);
    } else if node.is_missing() {
        let expected = node.kind().to_owned();
        diagnostics.push(make_diagnostic(
            source,
            node,
            ParseIssueKind::Missing {
                expected: expected.clone(),
            },
            format!("missing {expected}"),
        )?);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_parse_diagnostics(child, source, diagnostics)?;
    }

    Ok(())
}

fn make_diagnostic(
    source: &SourceFile,
    node: Node<'_>,
    kind: ParseIssueKind,
    message: String,
) -> Result<ParseDiagnostic, PositionError> {
    let range = ByteRange::new(node.start_byte(), node.end_byte());

    Ok(ParseDiagnostic {
        kind,
        range,
        start: source.position(range.start)?,
        end: source.position(range.end)?,
        message,
    })
}

#[derive(Debug)]
pub enum ParserError {
    Language(LanguageError),
    Cancelled,
    Position(PositionError),
}

impl From<PositionError> for ParserError {
    fn from(error: PositionError) -> Self {
        Self::Position(error)
    }
}

impl fmt::Display for ParserError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Language(error) => write!(formatter, "failed to load MATLAB grammar: {error}"),
            Self::Cancelled => write!(formatter, "MATLAB parser returned no syntax tree"),
            Self::Position(error) => write!(formatter, "invalid parser byte position: {error}"),
        }
    }
}

impl std::error::Error for ParserError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fixture(name: &str, text: &str) -> ParseResult {
        let source = SourceFile::new(name, text);
        MatlabParser::new()
            .expect("grammar should load")
            .parse(&source)
            .expect("parser should return a tree")
    }

    #[test]
    fn valid_source_allows_structural_formatting() {
        let result = parse_fixture("valid.m", "function y = twice(x)\n  y = 2 * x;\nend\n");

        assert!(!result.has_errors());
        assert!(result.diagnostics().is_empty());
        assert!(result.structural_formatting_allowed());
    }

    #[test]
    fn probe_missing_recovery_candidates() {
        let candidates = [
            ("unclosed_paren", "x = (1 + 2\n"),
            ("missing_end", "if true\n  x = 1;\n"),
            ("missing_condition", "if\nend\n"),
            ("missing_rhs", "x = 1 + ;\n"),
            ("unclosed_matrix", "x = [1 2\n"),
            ("unclosed_cell", "x = {1, 2\n"),
            ("unclosed_call", "x = foo(1, 2\n"),
            ("double_comma", "x = foo(1,,2);\n"),
            ("missing_for_end", "for i = 1:3\n  x = i;\n"),
            ("missing_function_end", "function y = f(x)\n  y = x;\n"),
            ("missing_switch_end", "switch x\ncase 1\n  y = 1;\n"),
        ];

        let mut found = Vec::new();
        let mut trees = Vec::new();

        for (name, text) in candidates {
            let result = parse_fixture(name, text);
            if result
                .diagnostics()
                .iter()
                .any(|diagnostic| matches!(diagnostic.kind, ParseIssueKind::Missing { .. }))
            {
                found.push(name);
            }
            trees.push((name, result.tree().root_node().to_sexp()));
        }

        panic!("missing-node candidates: {found:?}; trees: {trees:#?}");
    }

    #[test]
    fn missing_recovery_is_reported_and_blocks_structural_formatting() {
        let source = include_str!("../tests/fixtures/parser/malformed_missing.m");
        let result = parse_fixture("malformed_missing.m", source);

        assert!(result.has_errors());
        assert!(!result.structural_formatting_allowed());
        assert!(
            result
                .diagnostics()
                .iter()
                .any(|diagnostic| matches!(diagnostic.kind, ParseIssueKind::Missing { .. }))
        );
    }

    #[test]
    fn explicit_error_is_reported_and_blocks_structural_formatting() {
        let source = include_str!("../tests/fixtures/parser/malformed_error.m");
        let result = parse_fixture("malformed_error.m", source);

        assert!(result.has_errors());
        assert!(!result.structural_formatting_allowed());
        assert!(
            result
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.kind == ParseIssueKind::Error)
        );
    }
}
