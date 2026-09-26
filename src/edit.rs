use std::fmt;

use crate::source::ByteRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    pub range: ByteRange,
    pub replacement: String,
    pub rule_id: String,
}

impl Edit {
    pub fn new(
        start_byte: usize,
        end_byte: usize,
        replacement: impl Into<String>,
        rule_id: impl Into<String>,
    ) -> Self {
        Self {
            range: ByteRange::new(start_byte, end_byte),
            replacement: replacement.into(),
            rule_id: rule_id.into(),
        }
    }

    pub fn is_insertion(&self) -> bool {
        self.range.is_empty()
    }

    fn same_effect(&self, other: &Self) -> bool {
        self.range == other.range && self.replacement == other.replacement
    }
}

pub fn normalize_edits(source_len: usize, mut edits: Vec<Edit>) -> Result<Vec<Edit>, EditError> {
    for edit in &edits {
        if !edit.range.is_valid_for(source_len) {
            return Err(EditError::InvalidRange {
                edit: Box::new(edit.clone()),
                source_len,
            });
        }
    }

    edits.sort_by(|left, right| {
        (
            left.range.start,
            left.range.end,
            &left.replacement,
            &left.rule_id,
        )
            .cmp(&(
                right.range.start,
                right.range.end,
                &right.replacement,
                &right.rule_id,
            ))
    });

    let mut normalized: Vec<Edit> = Vec::with_capacity(edits.len());

    for edit in edits {
        if let Some(previous) = normalized.last() {
            if previous.same_effect(&edit) {
                continue;
            }

            if edits_conflict(previous, &edit) {
                return Err(EditError::Conflict {
                    first: Box::new(previous.clone()),
                    second: Box::new(edit),
                });
            }
        }

        normalized.push(edit);
    }

    Ok(normalized)
}

pub fn apply_edits(source: &str, edits: Vec<Edit>) -> Result<String, EditError> {
    for edit in &edits {
        if !source.is_char_boundary(edit.range.start) || !source.is_char_boundary(edit.range.end) {
            return Err(EditError::InvalidUtf8Boundary {
                edit: Box::new(edit.clone()),
            });
        }
    }

    let edits = normalize_edits(source.len(), edits)?;
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;

    for edit in edits {
        output.push_str(&source[cursor..edit.range.start]);
        output.push_str(&edit.replacement);
        cursor = edit.range.end;
    }

    output.push_str(&source[cursor..]);
    Ok(output)
}

fn edits_conflict(left: &Edit, right: &Edit) -> bool {
    debug_assert!(left.range.start <= right.range.start);

    if left.range.start == right.range.start && (left.is_insertion() || right.is_insertion()) {
        return true;
    }

    if left.range.end > right.range.start {
        return true;
    }

    left.range.end == right.range.start && right.is_insertion() && !left.is_insertion()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    InvalidRange { edit: Box<Edit>, source_len: usize },
    InvalidUtf8Boundary { edit: Box<Edit> },
    Conflict { first: Box<Edit>, second: Box<Edit> },
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRange { edit, source_len } => write!(
                formatter,
                "edit {}..{} from {} is invalid for source length {}",
                edit.range.start, edit.range.end, edit.rule_id, source_len
            ),
            Self::InvalidUtf8Boundary { edit } => write!(
                formatter,
                "edit {}..{} from {} is not aligned to UTF-8 character boundaries",
                edit.range.start, edit.range.end, edit.rule_id
            ),
            Self::Conflict { first, second } => write!(
                formatter,
                "conflicting edits from {} ({}..{}) and {} ({}..{})",
                first.rule_id,
                first.range.start,
                first.range.end,
                second.rule_id,
                second.range.start,
                second.range.end
            ),
        }
    }
}

impl std::error::Error for EditError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_disjoint_edits_in_source_order() {
        let source = "abcdef";
        let output = apply_edits(
            source,
            vec![Edit::new(4, 6, "EF", "F002"), Edit::new(0, 2, "AB", "F001")],
        )
        .expect("disjoint edits should apply");

        assert_eq!(output, "ABcdEF");
    }

    #[test]
    fn rejects_edits_inside_utf8_scalars() {
        let error = apply_edits("é", vec![Edit::new(1, 1, "x", "F001")])
            .expect_err("UTF-8 interior offset must fail");

        assert!(matches!(error, EditError::InvalidUtf8Boundary { .. }));
    }

    #[test]
    fn permits_touching_replacements() {
        let edits = normalize_edits(
            6,
            vec![
                Edit::new(0, 3, "ABC", "F001"),
                Edit::new(3, 6, "DEF", "F002"),
            ],
        )
        .expect("touching replacements are deterministic");

        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn rejects_overlapping_replacements() {
        let error = normalize_edits(
            6,
            vec![Edit::new(0, 4, "x", "F001"), Edit::new(3, 5, "y", "F002")],
        )
        .expect_err("overlap must fail");

        assert!(matches!(error, EditError::Conflict { .. }));
    }

    #[test]
    fn rejects_distinct_insertions_at_same_boundary() {
        let error = normalize_edits(
            6,
            vec![Edit::new(3, 3, "x", "F001"), Edit::new(3, 3, "y", "F002")],
        )
        .expect_err("same-position insertions are order-dependent");

        assert!(matches!(error, EditError::Conflict { .. }));
    }

    #[test]
    fn rejects_insertion_on_replacement_boundary() {
        let error = normalize_edits(
            6,
            vec![Edit::new(0, 3, "ABC", "F001"), Edit::new(3, 3, "x", "F002")],
        )
        .expect_err("boundary insertion must fail");

        assert!(matches!(error, EditError::Conflict { .. }));
    }

    #[test]
    fn deduplicates_identical_effects() {
        let edits = normalize_edits(
            6,
            vec![Edit::new(2, 2, "x", "F002"), Edit::new(2, 2, "x", "F001")],
        )
        .expect("identical effects may be coalesced");

        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].replacement, "x");
    }
}
