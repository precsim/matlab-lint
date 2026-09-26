use crate::source::ByteRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AmbiguityKind {
    CompactNumericDotOperator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AmbiguityZone {
    pub range: ByteRange,
    pub kind: AmbiguityKind,
}

pub fn find_ambiguity_zones(source: &str) -> Vec<AmbiguityZone> {
    let bytes = source.as_bytes();
    let mut zones = Vec::new();

    for dot in 1..bytes.len().saturating_sub(1) {
        if bytes[dot] != b'.'
            || !bytes[dot - 1].is_ascii_digit()
            || !matches!(bytes[dot + 1], b'/' | b'\\' | b'*' | b'^')
        {
            continue;
        }

        let mut start = dot;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }

        zones.push(AmbiguityZone {
            range: ByteRange::new(start, dot + 2),
            kind: AmbiguityKind::CompactNumericDotOperator,
        });
    }

    zones
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_compact_numeric_dot_operator_forms() {
        let source = include_str!("../tests/fixtures/parser/compact_dot_operator.m");
        let zones = find_ambiguity_zones(source);

        assert_eq!(zones.len(), 1);
        assert_eq!(&source[zones[0].range.start..zones[0].range.end], "1./");
    }

    #[test]
    fn ordinary_elementwise_operators_are_not_flagged() {
        assert!(find_ambiguity_zones("y = A./b;\nz = 1. / b;\n").is_empty());
    }
}
