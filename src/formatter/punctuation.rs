use tree_sitter::Node;

use crate::edit::Edit;
use crate::preserve::{PreservationZone, RuleCategory, edit_allowed};
use crate::source::ByteRange;

pub(crate) fn collect_edits(
    root: Node<'_>,
    source: &str,
    zones: &[PreservationZone],
) -> Vec<Edit> {
    let mut edits = Vec::new();
    visit(root, source, zones, &mut edits);
    edits
}

fn visit(
    node: Node<'_>,
    source: &str,
    zones: &[PreservationZone],
    edits: &mut Vec<Edit>,
) {
    match node.kind() {
        "," if token_allowed(node, zones) => format_comma(node, source, zones, edits),
        "=" if assignment_equals(node) && token_allowed(node, zones) => {
            normalize_space_before(node.start_byte(), source, zones, "F007", edits);
            normalize_space_after(node.end_byte(), source, zones, "F007", edits);
        }
        ";" if token_allowed(node, zones) => {
            remove_space_before(node.start_byte(), source, zones, "F008", edits);
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, source, zones, edits);
    }
}

fn format_comma(
    node: Node<'_>,
    source: &str,
    zones: &[PreservationZone],
    edits: &mut Vec<Edit>,
) {
    remove_space_before(node.start_byte(), source, zones, "F006", edits);

    let bytes = source.as_bytes();
    let end = node.end_byte();
    let after = horizontal_end(bytes, end);

    if after >= bytes.len() || matches!(bytes[after], b'\n' | b'\r') {
        return;
    }

    let existing = &source[end..after];
    if existing == " " {
        return;
    }

    let range = ByteRange::new(end, after);
    if edit_allowed(RuleCategory::Structural, range, zones) {
        edits.push(Edit::new(range.start, range.end, " ", "F006"));
    }
}

fn normalize_space_before(
    position: usize,
    source: &str,
    zones: &[PreservationZone],
    rule_id: &str,
    edits: &mut Vec<Edit>,
) {
    if position == 0 {
        return;
    }

    let bytes = source.as_bytes();
    let start = horizontal_start(bytes, position);
    if start == position && matches!(bytes[position - 1], b'\n' | b'\r') {
        return;
    }

    let existing = &source[start..position];
    if existing == " " {
        return;
    }

    let range = ByteRange::new(start, position);
    if edit_allowed(RuleCategory::Structural, range, zones) {
        edits.push(Edit::new(range.start, range.end, " ", rule_id));
    }
}

fn normalize_space_after(
    position: usize,
    source: &str,
    zones: &[PreservationZone],
    rule_id: &str,
    edits: &mut Vec<Edit>,
) {
    let bytes = source.as_bytes();
    if position >= bytes.len() {
        return;
    }

    let end = horizontal_end(bytes, position);
    if end >= bytes.len() || matches!(bytes[end], b'\n' | b'\r') {
        return;
    }

    let existing = &source[position..end];
    if existing == " " {
        return;
    }

    let range = ByteRange::new(position, end);
    if edit_allowed(RuleCategory::Structural, range, zones) {
        edits.push(Edit::new(range.start, range.end, " ", rule_id));
    }
}

fn remove_space_before(
    position: usize,
    source: &str,
    zones: &[PreservationZone],
    rule_id: &str,
    edits: &mut Vec<Edit>,
) {
    let bytes = source.as_bytes();
    let start = horizontal_start(bytes, position);
    if start == position {
        return;
    }

    let range = ByteRange::new(start, position);
    if edit_allowed(RuleCategory::Structural, range, zones) {
        edits.push(Edit::new(range.start, range.end, "", rule_id));
    }
}

fn horizontal_start(bytes: &[u8], position: usize) -> usize {
    let mut cursor = position;
    while cursor > 0 && matches!(bytes[cursor - 1], b' ' | b'\t') {
        cursor -= 1;
    }
    cursor
}

fn horizontal_end(bytes: &[u8], position: usize) -> usize {
    let mut cursor = position;
    while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
        cursor += 1;
    }
    cursor
}

fn token_allowed(node: Node<'_>, zones: &[PreservationZone]) -> bool {
    edit_allowed(
        RuleCategory::Structural,
        ByteRange::new(node.start_byte(), node.end_byte()),
        zones,
    )
}

fn assignment_equals(node: Node<'_>) -> bool {
    node.parent().is_some_and(|parent| {
        matches!(
            parent.kind(),
            "assignment" | "iterator" | "property" | "attribute" | "function_output"
        )
    })
}

#[cfg(test)]
mod tests {
    use crate::formatter::{FormatterOptions, format_source};
    use crate::source::SourceFile;

    #[test]
    fn formats_safe_punctuation_but_preserves_matrix_contents() {
        let source = SourceFile::new(
            "punctuation.m",
            "function y=f(x)\nA=[1,2;3,4];\ny=max(x,2) ;\nend\n",
        );
        let outcome = format_source(&source, FormatterOptions::default()).expect("format");

        assert_eq!(
            outcome.output(),
            "function y = f(x)\n  A = [1,2;3,4];\n  y = max(x, 2);\nend\n"
        );
    }
}
