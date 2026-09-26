use mstyle::formatter::{FormatterOptions, format_source};
use mstyle::parser::MatlabParser;
use mstyle::source::SourceFile;

struct Fixture {
    name: &'static str,
    input: &'static str,
    expected: &'static str,
    parses_cleanly: bool,
}

#[test]
fn formatter_golden_fixtures_are_exact_idempotent_and_parse_safe() {
    let fixtures = [
        Fixture {
            name: "basic",
            input: include_str!("fixtures/formatter/basic/input.m"),
            expected: include_str!("fixtures/formatter/basic/expected.m"),
            parses_cleanly: true,
        },
        Fixture {
            name: "preservation",
            input: include_str!("fixtures/formatter/preservation/input.m"),
            expected: include_str!("fixtures/formatter/preservation/expected.m"),
            parses_cleanly: true,
        },
        Fixture {
            name: "malformed",
            input: include_str!("fixtures/formatter/malformed/input.m"),
            expected: include_str!("fixtures/formatter/malformed/expected.m"),
            parses_cleanly: false,
        },
        Fixture {
            name: "runtime",
            input: include_str!("fixtures/formatter/runtime/input.m"),
            expected: include_str!("fixtures/formatter/runtime/expected.m"),
            parses_cleanly: true,
        },
        Fixture {
            name: "comparison",
            input: include_str!("fixtures/formatter/comparison/input.m"),
            expected: include_str!("fixtures/formatter/comparison/expected.m"),
            parses_cleanly: true,
        },
        Fixture {
            name: "binary",
            input: include_str!("fixtures/formatter/binary/input.m"),
            expected: include_str!("fixtures/formatter/binary/expected.m"),
            parses_cleanly: true,
        },
        Fixture {
            name: "range",
            input: include_str!("fixtures/formatter/range/input.m"),
            expected: include_str!("fixtures/formatter/range/expected.m"),
            parses_cleanly: true,
        },
    ];

    for fixture in fixtures {
        let source = SourceFile::new(format!("{}.m", fixture.name), fixture.input);
        let first = format_source(&source, FormatterOptions::default())
            .unwrap_or_else(|error| panic!("{} first pass failed: {error}", fixture.name));

        assert_eq!(
            first.output(),
            fixture.expected,
            "{} output did not match golden file",
            fixture.name
        );

        let expected_source =
            SourceFile::new(format!("{}-expected.m", fixture.name), fixture.expected);
        let second = format_source(&expected_source, FormatterOptions::default())
            .unwrap_or_else(|error| panic!("{} second pass failed: {error}", fixture.name));

        assert!(
            !second.changed(),
            "{} was not idempotent; second-pass edits: {:?}",
            fixture.name,
            second.edits()
        );

        let mut parser = MatlabParser::new().expect("grammar should load");
        let expected_parse = parser
            .parse(&expected_source)
            .unwrap_or_else(|error| panic!("{} expected parse failed: {error}", fixture.name));

        if fixture.parses_cleanly {
            assert!(
                !expected_parse.has_errors(),
                "{} expected output must parse cleanly",
                fixture.name
            );
        } else {
            assert!(
                first.has_parse_errors(),
                "{} malformed fixture must retain a parse diagnostic",
                fixture.name
            );
        }
    }
}
