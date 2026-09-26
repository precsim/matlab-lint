mod indentation;
mod lexical;
mod punctuation;

use std::fmt;

use crate::edit::{Edit, EditError, apply_edits, normalize_edits};
use crate::parser::{MatlabParser, ParseDiagnostic, ParserError};
use crate::preserve::collect_preservation_zones;
use crate::source::SourceFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::Crlf => "\r\n",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatterOptions {
    pub indent_width: usize,
    pub line_ending: LineEnding,
}

impl Default for FormatterOptions {
    fn default() -> Self {
        Self {
            indent_width: 2,
            line_ending: LineEnding::Lf,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FormatOutcome {
    output: String,
    edits: Vec<Edit>,
    parse_diagnostics: Vec<ParseDiagnostic>,
}

impl FormatOutcome {
    pub fn output(&self) -> &str {
        &self.output
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub fn parse_diagnostics(&self) -> &[ParseDiagnostic] {
        &self.parse_diagnostics
    }

    pub fn changed(&self) -> bool {
        !self.edits.is_empty()
    }

    pub fn has_parse_errors(&self) -> bool {
        !self.parse_diagnostics.is_empty()
    }
}

pub fn format_source(
    source: &SourceFile,
    options: FormatterOptions,
) -> Result<FormatOutcome, FormatterError> {
    let mut parser = MatlabParser::new()?;
    let parsed = parser.parse(source)?;
    let parse_diagnostics = parsed.diagnostics().to_vec();

    let mut edits = lexical::collect_edits(source.text(), options.line_ending);

    if parsed.structural_formatting_allowed() {
        let zones = collect_preservation_zones(parsed.tree().root_node(), source.text());
        edits.extend(indentation::collect_edits(
            parsed.tree().root_node(),
            source.text(),
            options.indent_width,
            &zones,
        ));
        edits.extend(punctuation::collect_edits(
            parsed.tree().root_node(),
            source.text(),
            &zones,
        ));
    }

    let edits = normalize_edits(source.len(), edits)?;
    let output = apply_edits(source.text(), edits.clone())?;

    if parsed.structural_formatting_allowed() && output != source.text() {
        let formatted = SourceFile::new(source.path(), output.clone());
        let reparsed = parser.parse(&formatted)?;

        if reparsed.has_errors() {
            return Err(FormatterError::SafetyViolation {
                message: "formatted source no longer parses cleanly".to_owned(),
            });
        }
    }

    Ok(FormatOutcome {
        output,
        edits,
        parse_diagnostics,
    })
}

#[derive(Debug)]
pub enum FormatterError {
    Parser(ParserError),
    Edit(EditError),
    SafetyViolation { message: String },
}

impl From<ParserError> for FormatterError {
    fn from(error: ParserError) -> Self {
        Self::Parser(error)
    }
}

impl From<EditError> for FormatterError {
    fn from(error: EditError) -> Self {
        Self::Edit(error)
    }
}

impl fmt::Display for FormatterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parser(error) => write!(formatter, "{error}"),
            Self::Edit(error) => write!(formatter, "{error}"),
            Self::SafetyViolation { message } => {
                write!(formatter, "formatter safety validation failed: {message}")
            }
        }
    }
}

impl std::error::Error for FormatterError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_source_only_receives_lexical_cleanup() {
        let source = SourceFile::new("broken.m", "x = 1 + ;   ");
        let outcome = format_source(&source, FormatterOptions::default()).expect("format");

        assert!(outcome.has_parse_errors());
        assert_eq!(outcome.output(), "x = 1 + ;\n");
        assert!(outcome.edits().iter().all(|edit| {
            edit.rule_id == "F001" || edit.rule_id == "F002" || edit.rule_id == "F003"
        }));
    }

    #[test]
    fn formatting_is_idempotent_for_basic_source() {
        let source = SourceFile::new(
            "basic.m",
            "function y=f(x)\nif x>0\ny=max(x,2) ;\nelse\ny=0;\nend\nend",
        );
        let options = FormatterOptions::default();
        let first = format_source(&source, options).expect("first format");
        let second_source = SourceFile::new("basic.m", first.output());
        let second = format_source(&second_source, options).expect("second format");

        assert!(first.changed());
        assert!(!second.changed());
    }
}
