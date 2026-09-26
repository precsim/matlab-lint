use tree_sitter::Node;

use crate::edit::Edit;
use crate::preserve::{PreservationReason, PreservationZone, RuleCategory, edit_allowed};
use crate::source::ByteRange;

const AMBIGUOUS_LEVEL: usize = usize::MAX;

#[derive(Debug, Clone, Copy)]
struct LineInfo {
    start: usize,
    content_end: usize,
    first_non_indent: usize,
}

pub(crate) fn collect_edits(
    root: Node<'_>,
    source: &str,
    indent_width: usize,
    zones: &[PreservationZone],
) -> Vec<Edit> {
    let lines = line_infos(source);
    if lines.is_empty() {
        return Vec::new();
    }

    let mut targets = vec![None; lines.len()];
    process_source(root, &mut targets);

    let mut continuation_rows = vec![false; lines.len()];
    for zone in zones {
        if zone.reason == PreservationReason::LineContinuation {
            let row = row_for_byte(&lines, zone.range.start);
            if row + 1 < continuation_rows.len() {
                continuation_rows[row + 1] = true;
            }
        }
    }

    let mut edits = Vec::new();

    for (row, line) in lines.iter().enumerate() {
        let Some(level) = targets[row] else {
            continue;
        };
        if level == AMBIGUOUS_LEVEL || continuation_rows[row] {
            continue;
        }
        if line.first_non_indent >= line.content_end {
            continue;
        }

        let current = &source[line.start..line.first_non_indent];
        let target = " ".repeat(level.saturating_mul(indent_width));
        if current == target {
            continue;
        }

        let range = ByteRange::new(line.start, line.first_non_indent);
        if !edit_allowed(RuleCategory::Structural, range, zones) {
            continue;
        }

        let rule_id = if current.as_bytes().contains(&b'\t') {
            "F004"
        } else {
            "F005"
        };
        edits.push(Edit::new(range.start, range.end, target, rule_id));
    }

    edits
}

fn process_source(root: Node<'_>, targets: &mut [Option<usize>]) {
    let mut cursor = root.walk();
    for child in root.named_children(&mut cursor) {
        process_statement(child, 0, targets);
    }
}

fn process_statement(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    match node.kind() {
        "if_statement" => process_if(node, level, targets),
        "for_statement" | "while_statement" | "spmd_statement" => {
            process_loop_like(node, level, targets)
        }
        "try_statement" => process_try(node, level, targets),
        "switch_statement" => process_switch(node, level, targets),
        "function_definition" => process_function(node, level, targets),
        "class_definition" => process_class(node, level, targets),
        "properties" | "methods" | "events" | "enumeration" => {
            process_section(node, level, targets)
        }
        "arguments_statement" => process_arguments(node, level, targets),
        "elseif_clause" | "else_clause" | "catch_clause" => process_clause(node, level, targets),
        "case_clause" | "otherwise_clause" => process_clause(node, level, targets),
        _ => mark_start(node, level, targets),
    }
}

fn process_block(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        process_statement(child, level, targets);
    }
}

fn process_if(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "block" => process_block(child, level + 1, targets),
            "elseif_clause" | "else_clause" => process_clause(child, level, targets),
            _ => {}
        }
    }
    mark_end(node, level, targets);
}

fn process_loop_like(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "block" {
            process_block(child, level + 1, targets);
        }
    }
    mark_end(node, level, targets);
}

fn process_try(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "block" => process_block(child, level + 1, targets),
            "catch_clause" => process_clause(child, level, targets),
            _ => {}
        }
    }
    mark_end(node, level, targets);
}

fn process_switch(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "case_clause" | "otherwise_clause" => process_clause(child, level + 1, targets),
            _ => {}
        }
    }
    mark_end(node, level, targets);
}

fn process_clause(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "block" {
            process_block(child, level + 1, targets);
        }
    }
}

fn process_function(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let start_row = node.start_position().row;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "arguments_statement" => process_arguments(child, level + 1, targets),
            "block" => process_block(child, level + 1, targets),
            "comment" if child.start_position().row > start_row => {
                mark_start(child, level + 1, targets)
            }
            _ => {}
        }
    }
    mark_end(node, level, targets);
}

fn process_class(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let start_row = node.start_position().row;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "properties" | "methods" | "events" | "enumeration" => {
                process_section(child, level + 1, targets)
            }
            "comment" if child.start_position().row > start_row => {
                mark_start(child, level + 1, targets)
            }
            _ => {}
        }
    }
    mark_end(node, level, targets);
}

fn process_section(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let start_row = node.start_position().row;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.start_position().row <= start_row {
            continue;
        }

        if child.kind() == "function_definition" {
            process_function(child, level + 1, targets);
        } else {
            mark_start(child, level + 1, targets);
        }
    }
    mark_end(node, level, targets);
}

fn process_arguments(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_start(node, level, targets);
    let start_row = node.start_position().row;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.start_position().row > start_row {
            mark_start(child, level + 1, targets);
        }
    }
    mark_end(node, level, targets);
}

fn mark_start(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    mark_row(node.start_position().row, level, targets);
}

fn mark_end(node: Node<'_>, level: usize, targets: &mut [Option<usize>]) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "end" | "endfunction") {
            mark_row(child.start_position().row, level, targets);
        }
    }
}

fn mark_row(row: usize, level: usize, targets: &mut [Option<usize>]) {
    let Some(slot) = targets.get_mut(row) else {
        return;
    };

    match *slot {
        None => *slot = Some(level),
        Some(existing) if existing == level => {}
        Some(_) => *slot = Some(AMBIGUOUS_LEVEL),
    }
}

fn line_infos(source: &str) -> Vec<LineInfo> {
    if source.is_empty() {
        return Vec::new();
    }

    let bytes = source.as_bytes();
    let mut starts = vec![0];
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' && index + 1 < bytes.len() {
            starts.push(index + 1);
        }
    }

    let mut lines = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().copied().enumerate() {
        let mut content_end = if index + 1 < starts.len() {
            starts[index + 1] - 1
        } else {
            bytes.len()
        };
        if content_end > start && bytes[content_end - 1] == b'\r' {
            content_end -= 1;
        }

        let mut first_non_indent = start;
        while first_non_indent < content_end && matches!(bytes[first_non_indent], b' ' | b'\t') {
            first_non_indent += 1;
        }

        lines.push(LineInfo {
            start,
            content_end,
            first_non_indent,
        });
    }
    lines
}

fn row_for_byte(lines: &[LineInfo], byte: usize) -> usize {
    match lines.binary_search_by_key(&byte, |line| line.start) {
        Ok(row) => row,
        Err(row) => row.saturating_sub(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formatter::{FormatterOptions, format_source};
    use crate::source::SourceFile;

    #[test]
    fn indents_switch_cases_and_bodies() {
        let source = SourceFile::new("switch.m", "switch x\ncase 1\ny=1;\notherwise\ny=2;\nend\n");
        let outcome = format_source(&source, FormatterOptions::default()).expect("format");

        assert_eq!(
            outcome.output(),
            "switch x\n  case 1\n    y = 1;\n  otherwise\n    y = 2;\nend\n"
        );
    }

    #[test]
    fn preserves_continuation_alignment() {
        let source = SourceFile::new(
            "continuation.m",
            "function y = f(x)\ny = x + ...\n        1;\nend\n",
        );
        let outcome = format_source(&source, FormatterOptions::default()).expect("format");

        assert!(outcome.output().contains("\n        1;\n"));
    }
}
