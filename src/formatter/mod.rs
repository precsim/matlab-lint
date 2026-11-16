mod indentation;
mod lexical;
mod punctuation;

use std::fmt;

use crate::edit::{Edit, EditError, apply_edits, normalize_edits};
use crate::parser::{MatlabParser, ParseDiagnostic, ParseResult, ParserError};
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
    pub rules: FormatterRuleSet,
}

impl Default for FormatterOptions {
    fn default() -> Self {
        Self {
            indent_width: 2,
            line_ending: LineEnding::Lf,
            rules: FormatterRuleSet::all(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatterRuleSet(u16);

impl FormatterRuleSet {
    const ALL_MASK: u16 = (1 << 11) - 1;

    pub const fn all() -> Self {
        Self(Self::ALL_MASK)
    }

    pub const fn none() -> Self {
        Self(0)
    }

    pub fn is_enabled(self, rule_id: &str) -> bool {
        rule_bit(rule_id).is_some_and(|bit| self.0 & bit != 0)
    }

    pub fn set_enabled(&mut self, rule_id: &str, enabled: bool) {
        let Some(bit) = rule_bit(rule_id) else {
            return;
        };

        if enabled {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }

    pub fn with_enabled(mut self, rule_id: &str, enabled: bool) -> Self {
        self.set_enabled(rule_id, enabled);
        self
    }
}

impl Default for FormatterRuleSet {
    fn default() -> Self {
        Self::all()
    }
}

fn rule_bit(rule_id: &str) -> Option<u16> {
    let index = match rule_id {
        "F001" => 0,
        "F002" => 1,
        "F003" => 2,
        "F004" => 3,
        "F005" => 4,
        "F006" => 5,
        "F007" => 6,
        "F008" => 7,
        "F009" => 8,
        "F010" => 9,
        "F011" => 10,
        _ => return None,
    };
    Some(1 << index)
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
    format_source_with_parse(source, options, &parsed)
}

pub fn format_source_with_parse(
    source: &SourceFile,
    options: FormatterOptions,
    parsed: &ParseResult,
) -> Result<FormatOutcome, FormatterError> {
    let parse_diagnostics = parsed.diagnostics().to_vec();
    let mut edits = collect_lexical_edits(source.text(), options);

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

    edits.retain(|edit| options.rules.is_enabled(&edit.rule_id));

    let edits = normalize_edits(source.len(), edits)?;
    let output = apply_edits(source.text(), edits.clone())?;

    if parsed.structural_formatting_allowed() && output != source.text() {
        let formatted = SourceFile::new(source.path(), output.clone());
        let mut parser = MatlabParser::new()?;
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

fn collect_lexical_edits(source: &str, options: FormatterOptions) -> Vec<Edit> {
    let trailing_whitespace = options.rules.is_enabled("F001");
    let eof_newline = options.rules.is_enabled("F002");
    let line_endings = options.rules.is_enabled("F003");

    lexical::collect_edits(source, options.line_ending)
        .into_iter()
        .filter_map(|edit| match edit.rule_id.as_str() {
            "F001" => trailing_whitespace.then_some(edit),
            "F002" => match (trailing_whitespace, eof_newline) {
                (true, true) => Some(edit),
                (false, true) => Some(Edit::new(
                    source.len(),
                    source.len(),
                    options.line_ending.as_str(),
                    "F002",
                )),
                (true, false) if !edit.range.is_empty() => {
                    Some(Edit::new(edit.range.start, edit.range.end, "", "F001"))
                }
                _ => None,
            },
            "F003" => line_endings.then_some(edit),
            _ => Some(edit),
        })
        .collect()
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

    #[test]
    fn formatter_rule_set_defaults_to_all_existing_rules() {
        let rules = FormatterRuleSet::default();

        for index in 1..=11 {
            assert!(rules.is_enabled(&format!("F{index:03}")));
        }
        assert!(!rules.is_enabled("L001"));
        assert!(!rules.is_enabled("F999"));
    }

    #[test]
    fn disabled_structural_rule_does_not_emit_or_apply_edits() {
        let source = SourceFile::new("assignment.m", "x=1;\n");
        let options = FormatterOptions {
            rules: FormatterRuleSet::all().with_enabled("F007", false),
            ..FormatterOptions::default()
        };
        let outcome = format_source(&source, options).expect("format");

        assert_eq!(outcome.output(), source.text());
        assert!(outcome.edits().iter().all(|edit| edit.rule_id != "F007"));
    }

    #[test]
    fn disabled_lexical_rules_are_suppressed_on_malformed_source() {
        let source = SourceFile::new("broken.m", "x = 1 + ;   ");
        let options = FormatterOptions {
            rules: FormatterRuleSet::none(),
            ..FormatterOptions::default()
        };
        let outcome = format_source(&source, options).expect("format");

        assert!(outcome.has_parse_errors());
        assert_eq!(outcome.output(), source.text());
        assert!(outcome.edits().is_empty());
    }

    #[test]
    fn trailing_whitespace_and_eof_newline_can_be_toggled_independently() {
        let source = SourceFile::new("lexical.m", "x = 1;   ");

        let without_trailing_cleanup = format_source(
            &source,
            FormatterOptions {
                rules: FormatterRuleSet::all().with_enabled("F001", false),
                ..FormatterOptions::default()
            },
        )
        .expect("format without F001");
        assert_eq!(without_trailing_cleanup.output(), "x = 1;   \n");
        assert!(
            without_trailing_cleanup
                .edits()
                .iter()
                .all(|edit| edit.rule_id != "F001")
        );

        let without_eof_newline = format_source(
            &source,
            FormatterOptions {
                rules: FormatterRuleSet::all().with_enabled("F002", false),
                ..FormatterOptions::default()
            },
        )
        .expect("format without F002");
        assert_eq!(without_eof_newline.output(), "x = 1;");
        assert!(
            without_eof_newline
                .edits()
                .iter()
                .all(|edit| edit.rule_id != "F002")
        );
    }
}
