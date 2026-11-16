use std::collections::{BTreeMap, BTreeSet};

use tree_sitter::Node;

use crate::diagnostic::{Diagnostic, Severity};
use crate::parser::ParseResult;
use crate::source::{ByteRange, PositionError, SourceFile};

#[derive(Debug, Clone, Copy)]
pub struct LintOptions {
    pub line_length: usize,
    pub line_length_severity: Severity,
    pub multiple_statements_severity: Severity,
    pub global_severity: Severity,
    pub eval_severity: Severity,
    pub wildcard_import_severity: Severity,
    pub file_name_mismatch_severity: Severity,
    pub duplicate_local_function_severity: Severity,
    pub duplicate_case_severity: Severity,
    pub unreachable_statement_severity: Severity,
    pub missing_semicolon_severity: Option<Severity>,
}

pub fn lint_source(
    source: &SourceFile,
    parsed: &ParseResult,
    options: LintOptions,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut diagnostics = line_length_diagnostics(source, options)?;

    if parsed.structural_formatting_allowed() {
        let root = parsed.tree().root_node();
        diagnostics.extend(multiple_statement_diagnostics(
            source,
            root,
            options.multiple_statements_severity,
        )?);
        diagnostics.extend(discouraged_global_diagnostics(
            source,
            root,
            options.global_severity,
        )?);
        diagnostics.extend(discouraged_eval_diagnostics(
            source,
            root,
            options.eval_severity,
        )?);
        diagnostics.extend(wildcard_import_diagnostics(
            source,
            root,
            options.wildcard_import_severity,
        )?);
        diagnostics.extend(primary_name_mismatch_diagnostics(
            source,
            root,
            options.file_name_mismatch_severity,
        )?);
        diagnostics.extend(duplicate_local_function_diagnostics(
            source,
            root,
            options.duplicate_local_function_severity,
        )?);
        diagnostics.extend(duplicate_case_diagnostics(
            source,
            root,
            options.duplicate_case_severity,
        )?);
        diagnostics.extend(unreachable_statement_diagnostics(
            source,
            root,
            options.unreachable_statement_severity,
        )?);
        if let Some(severity) = options.missing_semicolon_severity {
            diagnostics.extend(missing_semicolon_diagnostics(source, root, severity)?);
        }
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
    if matches!(node.kind(), "source_file" | "block") {
        let mut first_by_row = BTreeMap::<usize, usize>::new();
        let mut cursor = node.walk();

        for child in node.named_children(&mut cursor) {
            if matches!(child.kind(), "comment" | "line_continuation") {
                continue;
            }

            let row = child.start_position().row;
            if first_by_row.insert(row, child.start_byte()).is_some() {
                diagnostics
                    .entry(row)
                    .or_insert_with(|| ByteRange::new(child.start_byte(), child.end_byte()));
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_multiple_statements(child, diagnostics);
    }
}

fn make_diagnostic(
    source: &SourceFile,
    rule: &str,
    severity: Severity,
    range: ByteRange,
    message: impl Into<String>,
) -> Result<Diagnostic, PositionError> {
    Ok(Diagnostic {
        rule: rule.to_owned(),
        severity,
        file: source.path().to_string_lossy().into_owned(),
        start: source.position(range.start)?,
        end: source.position(range.end)?,
        start_byte: range.start,
        end_byte: range.end,
        message: message.into(),
        fixable: false,
    })
}

fn node_range(node: Node<'_>) -> ByteRange {
    ByteRange::new(node.start_byte(), node.end_byte())
}

fn discouraged_global_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut ranges = Vec::new();
    collect_kind_ranges(root, "global_operator", &mut ranges);
    ranges
        .into_iter()
        .map(|range| {
            make_diagnostic(
                source,
                "L003",
                severity,
                range,
                "global variables are discouraged",
            )
        })
        .collect()
}

fn collect_kind_ranges(node: Node<'_>, kind: &str, ranges: &mut Vec<ByteRange>) {
    if node.kind() == kind {
        ranges.push(node_range(node));
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_kind_ranges(child, kind, ranges);
    }
}

fn discouraged_eval_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut uses = Vec::<(ByteRange, &'static str)>::new();
    collect_eval_uses(root, source, &mut uses)?;
    uses.into_iter()
        .map(|(range, name)| {
            make_diagnostic(
                source,
                "L004",
                severity,
                range,
                format!("dynamic evaluation with `{name}` is discouraged"),
            )
        })
        .collect()
}

fn collect_eval_uses(
    node: Node<'_>,
    source: &SourceFile,
    uses: &mut Vec<(ByteRange, &'static str)>,
) -> Result<(), PositionError> {
    match node.kind() {
        "command" => {
            if let Some(name_node) = named_child_of_kind(node, "command_name") {
                let name = source.slice(node_range(name_node))?;
                if let Some(target) = discouraged_eval_name(name) {
                    uses.push((node_range(name_node), target));
                }
            }
        }
        "function_call" if !inside_assignment_left(node) => {
            if let Some(name_node) = node.child_by_field_name("name")
                && name_node.kind() == "identifier"
            {
                let name = source.slice(node_range(name_node))?;
                if let Some(target) = discouraged_eval_name(name)
                    && let Some(scope) = enclosing_scope(node)
                    && !scope_shadows_name(scope, target, source)?
                {
                    uses.push((node_range(name_node), target));
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_eval_uses(child, source, uses)?;
    }
    Ok(())
}

fn discouraged_eval_name(name: &str) -> Option<&'static str> {
    match name {
        "eval" => Some("eval"),
        "evalin" => Some("evalin"),
        _ => None,
    }
}

fn named_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn inside_assignment_left(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == "assignment" {
            return parent.child_by_field_name("left").is_some_and(|left| {
                left.start_byte() <= node.start_byte() && node.end_byte() <= left.end_byte()
            });
        }
        if matches!(parent.kind(), "function_definition" | "source_file") {
            return false;
        }
        node = parent;
    }
    false
}

fn enclosing_scope<'tree>(mut node: Node<'tree>) -> Option<Node<'tree>> {
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "function_definition" | "source_file") {
            return Some(parent);
        }
        node = parent;
    }
    None
}

fn scope_shadows_name(
    scope: Node<'_>,
    name: &str,
    source: &SourceFile,
) -> Result<bool, PositionError> {
    let root_function = (scope.kind() == "function_definition").then_some(scope.start_byte());
    subtree_binds_name(scope, root_function, name, source)
}

fn subtree_binds_name(
    node: Node<'_>,
    root_function: Option<usize>,
    name: &str,
    source: &SourceFile,
) -> Result<bool, PositionError> {
    if node.kind() == "function_definition" {
        let is_root = root_function == Some(node.start_byte());
        if function_name_matches(node, name, source)? {
            return Ok(true);
        }
        if !is_root {
            return Ok(false);
        }
    }

    let binds = match node.kind() {
        "assignment" => match node.child_by_field_name("left") {
            Some(left) => binding_target_matches(left, name, source)?,
            None => false,
        },
        "iterator" => match named_child_of_kind(node, "identifier") {
            Some(identifier) => node_text_matches(identifier, name, source)?,
            None => false,
        },
        "global_operator" | "persistent_operator" => {
            immediate_identifier_matches(node, name, source)?
        }
        "function_arguments" | "function_output" => subtree_identifier_matches(node, name, source)?,
        "catch_clause" => match named_child_of_kind(node, "identifier") {
            Some(identifier) => node_text_matches(identifier, name, source)?,
            None => false,
        },
        _ => false,
    };
    if binds {
        return Ok(true);
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if subtree_binds_name(child, root_function, name, source)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn function_name_matches(
    node: Node<'_>,
    name: &str,
    source: &SourceFile,
) -> Result<bool, PositionError> {
    let Some(name_node) = node.child_by_field_name("name") else {
        return Ok(false);
    };
    node_text_matches(name_node, name, source)
}

fn node_text_matches(
    node: Node<'_>,
    expected: &str,
    source: &SourceFile,
) -> Result<bool, PositionError> {
    Ok(source.slice(node_range(node))? == expected)
}

fn immediate_identifier_matches(
    node: Node<'_>,
    name: &str,
    source: &SourceFile,
) -> Result<bool, PositionError> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "identifier" && node_text_matches(child, name, source)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn subtree_identifier_matches(
    node: Node<'_>,
    name: &str,
    source: &SourceFile,
) -> Result<bool, PositionError> {
    if node.kind() == "identifier" && node_text_matches(node, name, source)? {
        return Ok(true);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if subtree_identifier_matches(child, name, source)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn binding_target_matches(
    node: Node<'_>,
    name: &str,
    source: &SourceFile,
) -> Result<bool, PositionError> {
    match node.kind() {
        "identifier" => node_text_matches(node, name, source),
        "function_call" => match node.child_by_field_name("name") {
            Some(target) if target.kind() == "identifier" => {
                node_text_matches(target, name, source)
            }
            _ => Ok(false),
        },
        "field_expression" => match node.child_by_field_name("object") {
            Some(object) => binding_target_matches(object, name, source),
            None => Ok(false),
        },
        "multioutput_variable" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if binding_target_matches(child, name, source)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn wildcard_import_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut ranges = Vec::new();
    collect_wildcard_imports(root, source, &mut ranges)?;
    ranges
        .into_iter()
        .map(|range| {
            make_diagnostic(
                source,
                "L005",
                severity,
                range,
                "wildcard import is discouraged",
            )
        })
        .collect()
}

fn collect_wildcard_imports(
    node: Node<'_>,
    source: &SourceFile,
    ranges: &mut Vec<ByteRange>,
) -> Result<(), PositionError> {
    if node.kind() == "command"
        && let Some(command_name) = named_child_of_kind(node, "command_name")
        && node_text_matches(command_name, "import", source)?
    {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "command_argument" {
                let argument = source.slice(node_range(child))?.trim();
                if is_wildcard_import_argument(argument) {
                    ranges.push(node_range(child));
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_wildcard_imports(child, source, ranges)?;
    }
    Ok(())
}

fn is_wildcard_import_argument(argument: &str) -> bool {
    let Some(prefix) = argument.strip_suffix(".*") else {
        return false;
    };
    !prefix.is_empty()
        && prefix.split('.').all(|segment| {
            let mut characters = segment.chars();
            characters.next().is_some_and(|first| first.is_alphabetic())
                && characters.all(|character| character.is_alphanumeric() || character == '_')
        })
}

fn primary_name_mismatch_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let Some(file_stem) = source.path().file_stem().and_then(|stem| stem.to_str()) else {
        return Ok(Vec::new());
    };
    let Some(primary) = first_top_level_definition(root) else {
        return Ok(Vec::new());
    };
    let Some(name_node) = primary.child_by_field_name("name") else {
        return Ok(Vec::new());
    };
    let name = source.slice(node_range(name_node))?;
    if name == file_stem {
        return Ok(Vec::new());
    }

    Ok(vec![make_diagnostic(
        source,
        "L006",
        severity,
        node_range(name_node),
        format!(
            "primary {} name `{name}` does not match file name `{file_stem}`",
            if primary.kind() == "class_definition" {
                "class"
            } else {
                "function"
            }
        ),
    )?])
}

fn first_top_level_definition(root: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = root.walk();
    let first = root
        .named_children(&mut cursor)
        .find(|child| !matches!(child.kind(), "comment" | "line_continuation"))?;
    match first.kind() {
        "function_definition" | "class_definition" => Some(first),
        "block" => {
            let mut block_cursor = first.walk();
            first
                .named_children(&mut block_cursor)
                .find(|child| !matches!(child.kind(), "comment" | "line_continuation"))
                .filter(|child| matches!(child.kind(), "function_definition" | "class_definition"))
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FunctionOwner {
    File,
    Function(usize),
}

fn duplicate_local_function_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut seen = BTreeSet::<(FunctionOwner, String)>::new();
    let mut duplicates = Vec::<(ByteRange, String)>::new();
    collect_duplicate_local_functions(root, source, &mut seen, &mut duplicates)?;
    duplicates
        .into_iter()
        .map(|(range, name)| {
            make_diagnostic(
                source,
                "L007",
                severity,
                range,
                format!("duplicate local function name `{name}` in the same scope"),
            )
        })
        .collect()
}

fn collect_duplicate_local_functions(
    node: Node<'_>,
    source: &SourceFile,
    seen: &mut BTreeSet<(FunctionOwner, String)>,
    duplicates: &mut Vec<(ByteRange, String)>,
) -> Result<(), PositionError> {
    if node.kind() == "function_definition"
        && let Some(owner) = local_function_owner(node)
        && let Some(name_node) = node.child_by_field_name("name")
    {
        let name = source.slice(node_range(name_node))?.to_owned();
        if !seen.insert((owner, name.clone())) {
            duplicates.push((node_range(name_node), name));
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_duplicate_local_functions(child, source, seen, duplicates)?;
    }
    Ok(())
}

fn local_function_owner(mut node: Node<'_>) -> Option<FunctionOwner> {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "function_definition" => return Some(FunctionOwner::Function(parent.start_byte())),
            "class_definition" => return None,
            "source_file" => return Some(FunctionOwner::File),
            _ => node = parent,
        }
    }
    None
}

fn duplicate_case_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut duplicates = Vec::<(ByteRange, String)>::new();
    collect_duplicate_cases(root, source, &mut duplicates)?;
    duplicates
        .into_iter()
        .map(|(range, literal)| {
            make_diagnostic(
                source,
                "L008",
                severity,
                range,
                format!("duplicate literal case `{literal}` in the same switch"),
            )
        })
        .collect()
}

fn collect_duplicate_cases(
    node: Node<'_>,
    source: &SourceFile,
    duplicates: &mut Vec<(ByteRange, String)>,
) -> Result<(), PositionError> {
    if node.kind() == "switch_statement" {
        let mut seen = BTreeSet::<String>::new();
        let mut cursor = node.walk();
        for case in node
            .named_children(&mut cursor)
            .filter(|child| child.kind() == "case_clause")
        {
            let Some(condition) = case.child_by_field_name("condition") else {
                continue;
            };
            if !is_literal_case_condition(condition) {
                continue;
            }
            let literal = source.slice(node_range(condition))?.to_owned();
            if !seen.insert(literal.clone()) {
                duplicates.push((node_range(condition), literal));
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_duplicate_cases(child, source, duplicates)?;
    }
    Ok(())
}

fn is_literal_case_condition(node: Node<'_>) -> bool {
    match node.kind() {
        "number" | "string" => true,
        "parenthesis" => {
            let mut cursor = node.walk();
            let mut children = node.named_children(&mut cursor);
            children.next().is_some_and(is_literal_case_condition) && children.next().is_none()
        }
        "unary_operator" => node
            .child_by_field_name("operand")
            .is_some_and(is_literal_case_condition),
        _ => false,
    }
}

fn unreachable_statement_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut ranges = Vec::new();
    collect_unreachable_statements(root, &mut ranges);
    ranges
        .into_iter()
        .map(|range| {
            make_diagnostic(
                source,
                "L009",
                severity,
                range,
                "statement is unreachable after unconditional control transfer",
            )
        })
        .collect()
}

fn collect_unreachable_statements(node: Node<'_>, ranges: &mut Vec<ByteRange>) {
    if matches!(node.kind(), "block" | "source_file") {
        let mut terminated = false;
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if matches!(
                child.kind(),
                "comment" | "line_continuation" | "function_definition" | "class_definition"
            ) {
                continue;
            }
            if terminated {
                ranges.push(node_range(child));
                continue;
            }
            terminated = is_unconditional_transfer(child);
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_unreachable_statements(child, ranges);
    }
}

fn is_unconditional_transfer(node: Node<'_>) -> bool {
    match node.kind() {
        "return_statement" => true,
        "break_statement" | "continue_statement" => has_enclosing_loop(node),
        _ => false,
    }
}

fn has_enclosing_loop(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "for_statement" | "while_statement" => return true,
            "function_definition" | "source_file" => return false,
            _ => node = parent,
        }
    }
    false
}

fn missing_semicolon_diagnostics(
    source: &SourceFile,
    root: Node<'_>,
    severity: Severity,
) -> Result<Vec<Diagnostic>, PositionError> {
    let mut ranges = Vec::new();
    collect_missing_semicolons(root, source.text(), &mut ranges);
    ranges
        .into_iter()
        .map(|range| {
            make_diagnostic(
                source,
                "L010",
                severity,
                range,
                "assignment in a function should end with a semicolon",
            )
        })
        .collect()
}

fn collect_missing_semicolons(node: Node<'_>, source: &str, ranges: &mut Vec<ByteRange>) {
    if node.kind() == "assignment"
        && has_function_ancestor(node)
        && assignment_has_bare_line_ending(node, source)
    {
        ranges.push(node_range(node));
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_missing_semicolons(child, source, ranges);
    }
}

fn has_function_ancestor(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "function_definition" => return true,
            "source_file" => return false,
            _ => node = parent,
        }
    }
    false
}

fn assignment_has_bare_line_ending(node: Node<'_>, source: &str) -> bool {
    let tail = &source[node.end_byte()..];
    let line_tail = tail.split_once('\n').map_or(tail, |(line, _)| line);
    let remainder = line_tail.trim_start_matches([' ', '\t', '\r']);
    remainder.is_empty() || remainder.starts_with('%')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::MatlabParser;

    fn options(line_length: usize) -> LintOptions {
        LintOptions {
            line_length,
            line_length_severity: Severity::Warning,
            multiple_statements_severity: Severity::Warning,
            global_severity: Severity::Warning,
            eval_severity: Severity::Warning,
            wildcard_import_severity: Severity::Warning,
            file_name_mismatch_severity: Severity::Warning,
            duplicate_local_function_severity: Severity::Warning,
            duplicate_case_severity: Severity::Warning,
            unreachable_statement_severity: Severity::Warning,
            missing_semicolon_severity: None,
        }
    }

    fn lint_with_options(path: &str, text: &str, options: LintOptions) -> Vec<Diagnostic> {
        let source = SourceFile::new(path, text);
        let mut parser = MatlabParser::new().expect("grammar");
        let parsed = parser.parse(&source).expect("parse");
        lint_source(&source, &parsed, options).expect("lint")
    }

    fn lint(text: &str, line_length: usize) -> Vec<Diagnostic> {
        lint_with_options("lint.m", text, options(line_length))
    }

    fn rule_count(diagnostics: &[Diagnostic], rule: &str) -> usize {
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.rule == rule)
            .count()
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
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_discouraged_global_declarations() {
        let diagnostics = lint_with_options(
            "global_example.m",
            "function y = global_example()\n  global state\n  y = state;\nend\n",
            options(120),
        );
        assert_eq!(rule_count(&diagnostics, "L003"), 1);
    }

    #[test]
    fn reports_eval_and_evalin_but_skips_locally_shadowed_eval() {
        let diagnostics = lint_with_options(
            "dynamic_eval.m",
            concat!(
                "function y = dynamic_eval(code)\n",
                "  y = eval(code);\n",
                "  evalin('base', code);\n",
                "end\n",
            ),
            options(120),
        );
        assert_eq!(rule_count(&diagnostics, "L004"), 2);

        let shadowed = lint_with_options(
            "shadowed_eval.m",
            "function y = shadowed_eval(eval)\n  y = eval(1);\nend\n",
            options(120),
        );
        assert_eq!(rule_count(&shadowed, "L004"), 0);

        let assigned = lint_with_options(
            "assigned_eval.m",
            concat!(
                "function y = assigned_eval()\n",
                "  eval = [1, 2];\n",
                "  y = eval(1);\n",
                "end\n",
            ),
            options(120),
        );
        assert_eq!(rule_count(&assigned, "L004"), 0);
    }

    #[test]
    fn reports_only_wildcard_import_arguments() {
        let diagnostics = lint_with_options(
            "imports.m",
            "import package.tools.*\nimport package.tools.value\nx = 1;\n",
            options(120),
        );
        assert_eq!(rule_count(&diagnostics, "L005"), 1);
    }

    #[test]
    fn reports_primary_function_and_class_name_mismatches() {
        let function_diagnostics = lint_with_options(
            "wrong_name.m",
            "function y = actual_name(x)\n  y = x;\nend\n",
            options(120),
        );
        assert_eq!(rule_count(&function_diagnostics, "L006"), 1);

        let class_diagnostics =
            lint_with_options("WrongClass.m", "classdef ActualClass\nend\n", options(120));
        assert_eq!(rule_count(&class_diagnostics, "L006"), 1);
    }

    #[test]
    fn script_local_function_does_not_trigger_primary_name_mismatch() {
        let diagnostics = lint_with_options(
            "script.m",
            concat!("x = 1;\n", "function y = helper()\n", "  y = 1;\n", "end\n",),
            options(120),
        );
        assert_eq!(rule_count(&diagnostics, "L006"), 0);
    }

    #[test]
    fn reports_duplicate_local_function_names_in_same_owner() {
        let diagnostics = lint_with_options(
            "duplicate_functions.m",
            concat!(
                "function y = duplicate_functions(x)\n",
                "  y = helper(x);\n",
                "end\n",
                "function y = helper(x)\n",
                "  y = x;\n",
                "end\n",
                "function y = helper(x)\n",
                "  y = x + 1;\n",
                "end\n",
            ),
            options(120),
        );
        assert_eq!(rule_count(&diagnostics, "L007"), 1);
    }

    #[test]
    fn reports_only_exact_duplicate_literal_cases() {
        let diagnostics = lint_with_options(
            "duplicate_cases.m",
            concat!(
                "function y = duplicate_cases(x, token)\n",
                "  switch x\n",
                "    case 1\n",
                "      y = 1;\n",
                "    case 1\n",
                "      y = 2;\n",
                "    case token\n",
                "      y = 3;\n",
                "    case token\n",
                "      y = 4;\n",
                "    otherwise\n",
                "      y = 0;\n",
                "  end\n",
                "end\n",
            ),
            options(120),
        );
        assert_eq!(rule_count(&diagnostics, "L008"), 1);
    }

    #[test]
    fn reports_unreachable_siblings_after_return_break_and_continue() {
        let diagnostics = lint_with_options(
            "unreachable.m",
            concat!(
                "function y = unreachable(x)\n",
                "  y = 0;\n",
                "  if x > 0\n",
                "    return\n",
                "    y = 1;\n",
                "  end\n",
                "  for k = 1:3\n",
                "    continue\n",
                "    y = k;\n",
                "  end\n",
                "  while x < 0\n",
                "    break\n",
                "    y = 2;\n",
                "  end\n",
                "end\n",
            ),
            options(120),
        );
        assert_eq!(rule_count(&diagnostics, "L009"), 3);
    }

    #[test]
    fn conditional_return_does_not_make_following_outer_statement_unreachable() {
        let diagnostics = lint_with_options(
            "conditional_return.m",
            concat!(
                "function y = conditional_return(x)\n",
                "  if x > 0\n",
                "    return\n",
                "  end\n",
                "  y = 1;\n",
                "end\n",
            ),
            options(120),
        );
        assert_eq!(rule_count(&diagnostics, "L009"), 0);

        let invalid_break_scope = lint_with_options(
            "break_scope.m",
            "function y = break_scope()\n  break\n  y = 1;\nend\n",
            options(120),
        );
        assert_eq!(rule_count(&invalid_break_scope, "L009"), 0);
    }

    #[test]
    fn missing_semicolon_is_off_by_default_and_conservative_when_enabled() {
        let source = "function y = semicolon_rule(x)\n  y = x % display\n  z = x;\nend\n";
        let disabled = lint_with_options("semicolon_rule.m", source, options(120));
        assert_eq!(rule_count(&disabled, "L010"), 0);

        let mut enabled_options = options(120);
        enabled_options.missing_semicolon_severity = Some(Severity::Error);
        let enabled = lint_with_options("semicolon_rule.m", source, enabled_options);
        assert_eq!(rule_count(&enabled, "L010"), 1);
        assert_eq!(
            enabled
                .iter()
                .find(|diagnostic| diagnostic.rule == "L010")
                .expect("L010")
                .severity,
            Severity::Error
        );

        let script = lint_with_options("script.m", "x = 1\n", enabled_options);
        assert_eq!(rule_count(&script, "L010"), 0);
    }
}
