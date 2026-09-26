use tree_sitter::Node;

use crate::ambiguity::{AmbiguityKind, find_ambiguity_zones};
use crate::source::ByteRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PreservationReason {
    Matrix,
    Cell,
    String,
    Comment,
    Command,
    LineContinuation,
    ErrorRecovery,
    ParserAmbiguity(AmbiguityKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PreservationZone {
    pub range: ByteRange,
    pub reason: PreservationReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleCategory {
    LexicalCleanup,
    Structural,
}

pub fn collect_preservation_zones(root: Node<'_>, source: &str) -> Vec<PreservationZone> {
    let mut zones = Vec::new();
    collect_node_zones(root, &mut zones);

    zones.extend(
        find_ambiguity_zones(source)
            .into_iter()
            .map(|zone| PreservationZone {
                range: zone.range,
                reason: PreservationReason::ParserAmbiguity(zone.kind),
            }),
    );

    zones.sort();
    zones.dedup();
    zones
}

pub fn edit_allowed(category: RuleCategory, range: ByteRange, zones: &[PreservationZone]) -> bool {
    match category {
        RuleCategory::LexicalCleanup => true,
        RuleCategory::Structural => !zones
            .iter()
            .any(|zone| preservation_intersects(range, zone.range)),
    }
}

fn collect_node_zones(node: Node<'_>, zones: &mut Vec<PreservationZone>) {
    let reason = if node.is_error() || node.is_missing() {
        Some(PreservationReason::ErrorRecovery)
    } else {
        match node.kind() {
            "matrix" => Some(PreservationReason::Matrix),
            "cell" => Some(PreservationReason::Cell),
            "string" => Some(PreservationReason::String),
            "comment" => Some(PreservationReason::Comment),
            "command" => Some(PreservationReason::Command),
            "line_continuation" => Some(PreservationReason::LineContinuation),
            _ => None,
        }
    };

    if let Some(reason) = reason {
        zones.push(PreservationZone {
            range: ByteRange::new(node.start_byte(), node.end_byte()),
            reason,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node_zones(child, zones);
    }
}

fn preservation_intersects(edit: ByteRange, zone: ByteRange) -> bool {
    if edit.is_empty() {
        return zone.touches_point(edit.start);
    }

    if zone.is_empty() {
        return edit.touches_point(zone.start);
    }

    edit.overlaps(zone)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::MatlabParser;
    use crate::source::SourceFile;

    #[test]
    fn identifies_initial_preservation_zones() {
        let text = concat!(
            "A = [1 +1; 2 -3];\n",
            "C = {1, 'x'};\n",
            "s = \"text\"; % comment\n",
            "disp hello\n",
            "x = 1 + ...\n",
            "  2;\n",
            "q = 1./a;\n",
        );
        let source = SourceFile::new("zones.m", text);
        let mut parser = MatlabParser::new().expect("grammar should load");
        let parsed = parser.parse(&source).expect("source should parse");
        assert!(parsed.structural_formatting_allowed());

        let zones = collect_preservation_zones(parsed.tree().root_node(), source.text());
        let reasons: Vec<_> = zones.iter().map(|zone| zone.reason).collect();

        assert!(reasons.contains(&PreservationReason::Matrix));
        assert!(reasons.contains(&PreservationReason::Cell));
        assert!(reasons.contains(&PreservationReason::String));
        assert!(reasons.contains(&PreservationReason::Comment));
        assert!(reasons.contains(&PreservationReason::Command));
        assert!(reasons.contains(&PreservationReason::LineContinuation));
        assert!(reasons.contains(&PreservationReason::ParserAmbiguity(
            AmbiguityKind::CompactNumericDotOperator
        )));
    }

    #[test]
    fn structural_edits_are_blocked_but_lexical_cleanup_is_permitted() {
        let zones = vec![PreservationZone {
            range: ByteRange::new(5, 10),
            reason: PreservationReason::Comment,
        }];

        assert!(!edit_allowed(
            RuleCategory::Structural,
            ByteRange::new(6, 7),
            &zones
        ));
        assert!(!edit_allowed(
            RuleCategory::Structural,
            ByteRange::new(5, 5),
            &zones
        ));
        assert!(edit_allowed(
            RuleCategory::Structural,
            ByteRange::new(0, 4),
            &zones
        ));
        assert!(edit_allowed(
            RuleCategory::LexicalCleanup,
            ByteRange::new(6, 7),
            &zones
        ));
    }

    #[test]
    fn missing_zero_width_nodes_block_structural_edits_at_their_boundary() {
        let zones = vec![PreservationZone {
            range: ByteRange::new(5, 5),
            reason: PreservationReason::ErrorRecovery,
        }];

        assert!(!edit_allowed(
            RuleCategory::Structural,
            ByteRange::new(5, 5),
            &zones
        ));
        assert!(!edit_allowed(
            RuleCategory::Structural,
            ByteRange::new(4, 6),
            &zones
        ));
    }
}
