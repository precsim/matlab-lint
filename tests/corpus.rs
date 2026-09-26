use mstyle::formatter::{FormatterOptions, format_source};
use mstyle::parser::MatlabParser;
use mstyle::source::SourceFile;

struct CorpusCase {
    name: &'static str,
    source: &'static str,
}

#[test]
fn representative_matlab_corpus_parses_and_formats_idempotently() {
    let cases = [
        CorpusCase {
            name: "functions_and_control.m",
            source: include_str!("corpus/functions_and_control.m"),
        },
        CorpusCase {
            name: "switch_try.m",
            source: include_str!("corpus/switch_try.m"),
        },
        CorpusCase {
            name: "matrix_cell.m",
            source: include_str!("corpus/matrix_cell.m"),
        },
        CorpusCase {
            name: "continuation.m",
            source: include_str!("corpus/continuation.m"),
        },
        CorpusCase {
            name: "nested_function.m",
            source: include_str!("corpus/nested_function.m"),
        },
        CorpusCase {
            name: "class_definition.m",
            source: include_str!("corpus/class_definition.m"),
        },
        CorpusCase {
            name: "command_form.m",
            source: include_str!("corpus/command_form.m"),
        },
    ];

    for case in cases {
        let source = SourceFile::new(case.name, case.source);
        let mut parser = MatlabParser::new().expect("grammar should load");
        let parsed = parser
            .parse(&source)
            .unwrap_or_else(|error| panic!("{} parse failed: {error}", case.name));
        assert!(
            !parsed.has_errors(),
            "{} must parse cleanly: {:?}",
            case.name,
            parsed.diagnostics()
        );

        let first = format_source(&source, FormatterOptions::default())
            .unwrap_or_else(|error| panic!("{} format failed: {error}", case.name));
        let formatted = SourceFile::new(case.name, first.output());
        let second = format_source(&formatted, FormatterOptions::default())
            .unwrap_or_else(|error| panic!("{} second format failed: {error}", case.name));

        assert!(
            !second.changed(),
            "{} must be idempotent; second-pass edits: {:?}",
            case.name,
            second.edits()
        );
    }
}
