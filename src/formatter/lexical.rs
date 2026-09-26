use crate::edit::Edit;

use super::LineEnding;

pub(crate) fn collect_edits(source: &str, line_ending: LineEnding) -> Vec<Edit> {
    let bytes = source.as_bytes();
    let target = line_ending.as_str();
    let mut edits = Vec::new();
    let mut line_start = 0;
    let mut index = 0;

    while index < bytes.len() {
        let (newline_start, newline_end) = if bytes[index] == b'\r'
            && index + 1 < bytes.len()
            && bytes[index + 1] == b'\n'
        {
            (index, index + 2)
        } else if bytes[index] == b'\n' {
            (index, index + 1)
        } else {
            index += 1;
            continue;
        };

        let trailing_start = trim_horizontal_end(bytes, line_start, newline_start);
        if trailing_start < newline_start {
            edits.push(Edit::new(trailing_start, newline_start, "", "F001"));
        }

        let current = &source[newline_start..newline_end];
        if current != target {
            edits.push(Edit::new(newline_start, newline_end, target, "F003"));
        }

        line_start = newline_end;
        index = newline_end;
    }

    if !source.is_empty() && line_start < bytes.len() {
        let trailing_start = trim_horizontal_end(bytes, line_start, bytes.len());
        edits.push(Edit::new(
            trailing_start,
            bytes.len(),
            target,
            "F002",
        ));
    }

    edits
}

fn trim_horizontal_end(bytes: &[u8], start: usize, end: usize) -> usize {
    let mut cursor = end;
    while cursor > start && matches!(bytes[cursor - 1], b' ' | b'\t') {
        cursor -= 1;
    }
    cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(source: &str, line_ending: LineEnding) -> String {
        crate::edit::apply_edits(source, collect_edits(source, line_ending)).expect("apply")
    }

    #[test]
    fn removes_trailing_whitespace_and_adds_eof_newline() {
        assert_eq!(apply("x = 1;   ", LineEnding::Lf), "x = 1;\n");
    }

    #[test]
    fn normalizes_crlf_to_lf() {
        assert_eq!(
            apply("x = 1;  \r\ny = 2;\r\n", LineEnding::Lf),
            "x = 1;\ny = 2;\n"
        );
    }

    #[test]
    fn normalizes_lf_to_crlf() {
        assert_eq!(
            apply("x = 1;\ny = 2;\n", LineEnding::Crlf),
            "x = 1;\r\ny = 2;\r\n"
        );
    }

    #[test]
    fn empty_source_stays_empty() {
        assert_eq!(apply("", LineEnding::Lf), "");
    }
}
