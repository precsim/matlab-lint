use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteRange {
    pub start: usize,
    pub end: usize,
}

impl ByteRange {
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub const fn is_valid_for(self, source_len: usize) -> bool {
        self.start <= self.end && self.end <= source_len
    }

    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    pub const fn touches_point(self, point: usize) -> bool {
        self.start <= point && point <= self.end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SourcePosition {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    path: PathBuf,
    text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(path: impl Into<PathBuf>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];

        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }

        Self {
            path: path.into(),
            text,
            line_starts,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.text.len()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn slice(&self, range: ByteRange) -> Result<&str, PositionError> {
        self.validate_byte(range.start)?;
        self.validate_byte(range.end)?;

        if range.start > range.end {
            return Err(PositionError::InvalidRange(range));
        }

        Ok(&self.text[range.start..range.end])
    }

    pub fn position(&self, byte: usize) -> Result<SourcePosition, PositionError> {
        self.validate_byte(byte)?;

        let line_index = match self.line_starts.binary_search(&byte) {
            Ok(index) => index,
            Err(index) => index - 1,
        };
        let line_start = self.line_starts[line_index];
        let column = self.text[line_start..byte].chars().count() + 1;

        Ok(SourcePosition {
            line: line_index + 1,
            column,
        })
    }

    fn validate_byte(&self, byte: usize) -> Result<(), PositionError> {
        if byte > self.text.len() {
            return Err(PositionError::OutOfBounds {
                byte,
                source_len: self.text.len(),
            });
        }
        if !self.text.is_char_boundary(byte) {
            return Err(PositionError::NotCharBoundary { byte });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositionError {
    OutOfBounds { byte: usize, source_len: usize },
    NotCharBoundary { byte: usize },
    InvalidRange(ByteRange),
}

impl fmt::Display for PositionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfBounds { byte, source_len } => {
                write!(
                    formatter,
                    "byte offset {byte} exceeds source length {source_len}"
                )
            }
            Self::NotCharBoundary { byte } => {
                write!(
                    formatter,
                    "byte offset {byte} is not a UTF-8 character boundary"
                )
            }
            Self::InvalidRange(range) => {
                write!(
                    formatter,
                    "invalid byte range {}..{}",
                    range.start, range.end
                )
            }
        }
    }
}

impl std::error::Error for PositionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_utf8_bytes_to_one_based_scalar_positions() {
        let source = SourceFile::new("unicode.m", "aé\nβx\n");

        assert_eq!(
            source.position(0).expect("position"),
            SourcePosition { line: 1, column: 1 }
        );
        assert_eq!(
            source.position(1).expect("position"),
            SourcePosition { line: 1, column: 2 }
        );
        assert_eq!(
            source.position(3).expect("position"),
            SourcePosition { line: 1, column: 3 }
        );
        assert_eq!(
            source.position(4).expect("position"),
            SourcePosition { line: 2, column: 1 }
        );
        assert_eq!(
            source.position(6).expect("position"),
            SourcePosition { line: 2, column: 2 }
        );
    }

    #[test]
    fn rejects_positions_inside_utf8_scalars() {
        let source = SourceFile::new("unicode.m", "é");
        let error = source
            .position(1)
            .expect_err("middle of UTF-8 scalar must fail");

        assert_eq!(error, PositionError::NotCharBoundary { byte: 1 });
    }

    #[test]
    fn slices_only_valid_utf8_ranges() {
        let source = SourceFile::new("example.m", "alpha");
        assert_eq!(source.slice(ByteRange::new(1, 4)).expect("slice"), "lph");
        assert!(source.slice(ByteRange::new(4, 2)).is_err());
    }
}
