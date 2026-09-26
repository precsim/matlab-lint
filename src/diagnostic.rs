use std::path::Path;

use serde::Serialize;

use crate::edit::Edit;
use crate::parser::ParseDiagnostic;
use crate::source::{PositionError, SourceFile, SourcePosition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub rule: String,
    pub severity: Severity,
    pub file: String,
    pub start: SourcePosition,
    pub end: SourcePosition,
    pub start_byte: usize,
    pub end_byte: usize,
    pub message: String,
    pub fixable: bool,
}

impl Diagnostic {
    pub fn from_parse(path: &Path, diagnostic: &ParseDiagnostic) -> Self {
        Self {
            rule: "PARSE".to_owned(),
            severity: Severity::Error,
            file: path.to_string_lossy().into_owned(),
            start: diagnostic.start,
            end: diagnostic.end,
            start_byte: diagnostic.range.start,
            end_byte: diagnostic.range.end,
            message: diagnostic.message.clone(),
            fixable: false,
        }
    }

    pub fn from_formatter_edit(
        source: &SourceFile,
        edit: &Edit,
        severity: Severity,
    ) -> Result<Self, PositionError> {
        Ok(Self {
            rule: edit.rule_id.clone(),
            severity,
            file: source.path().to_string_lossy().into_owned(),
            start: source.position(edit.range.start)?,
            end: source.position(edit.range.end)?,
            start_byte: edit.range.start,
            end_byte: edit.range.end,
            message: formatter_message(&edit.rule_id).to_owned(),
            fixable: true,
        })
    }
}

pub fn sort_diagnostics(diagnostics: &mut [Diagnostic]) {
    diagnostics.sort_by(|left, right| {
        (
            &left.file,
            left.start_byte,
            left.end_byte,
            &left.rule,
            &left.message,
        )
            .cmp(&(
                &right.file,
                right.start_byte,
                right.end_byte,
                &right.rule,
                &right.message,
            ))
    });
}

fn formatter_message(rule_id: &str) -> &'static str {
    match rule_id {
        "F001" => "trailing whitespace",
        "F002" => "expected newline at end of file",
        "F003" => "line ending does not match configured style",
        "F004" => "tab in leading indentation",
        "F005" => "incorrect block indentation",
        "F006" => "incorrect comma spacing",
        "F007" => "incorrect assignment spacing",
        "F008" => "whitespace before semicolon",
        "F009" => "incorrect comparison/logical operator spacing",
        "F010" => "incorrect binary operator spacing",
        "F011" => "incorrect range colon spacing",
        _ => "formatting violation",
    }
}

#[derive(Debug, Serialize)]
pub struct DiagnosticReport<'a> {
    pub ok: bool,
    pub diagnostics: &'a [Diagnostic],
}

impl<'a> DiagnosticReport<'a> {
    pub fn new(diagnostics: &'a [Diagnostic]) -> Self {
        Self {
            ok: diagnostics.is_empty(),
            diagnostics,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ByteRange;

    #[test]
    fn formatter_edit_uses_stable_json_shape() {
        let source = SourceFile::new(
            "foo.m", "x=1;
",
        );
        let edit = Edit {
            range: ByteRange::new(1, 1),
            replacement: " ".to_owned(),
            rule_id: "F007".to_owned(),
        };
        let diagnostic =
            Diagnostic::from_formatter_edit(&source, &edit, Severity::Error).expect("diagnostic");
        let report = DiagnosticReport::new(std::slice::from_ref(&diagnostic));
        let json = serde_json::to_value(report).expect("serialize");

        assert_eq!(json["ok"], false);
        assert_eq!(json["diagnostics"][0]["rule"], "F007");
        assert_eq!(json["diagnostics"][0]["severity"], "error");
        assert_eq!(json["diagnostics"][0]["start"]["line"], 1);
        assert_eq!(json["diagnostics"][0]["start"]["column"], 2);
        assert_eq!(json["diagnostics"][0]["start_byte"], 1);
        assert_eq!(json["diagnostics"][0]["end_byte"], 1);
        assert_eq!(json["diagnostics"][0]["fixable"], true);
    }
}
