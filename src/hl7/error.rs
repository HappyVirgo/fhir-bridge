//! Error types for HL7 v2 parsing.

use std::fmt;

/// What went wrong while parsing an HL7 v2 message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// The input contained no segments at all.
    Empty,
    /// The first segment was not `MSH`.
    MissingMsh {
        /// The segment identifier (or line prefix) that was found instead.
        found: String,
    },
    /// The `MSH` segment is too short to contain the field separator and encoding characters.
    TruncatedMsh,
    /// `MSH-2` does not hold four or five distinct, non-alphanumeric ASCII characters.
    InvalidEncodingCharacters(String),
    /// A segment identifier is not three upper-case letters/digits starting with a letter.
    InvalidSegmentId(String),
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "input contains no segments"),
            Self::MissingMsh { found } => {
                write!(f, "message must start with an MSH segment, found {found:?}")
            }
            Self::TruncatedMsh => write!(f, "MSH segment is too short to declare its delimiters"),
            Self::InvalidEncodingCharacters(chars) => write!(
                f,
                "invalid encoding characters {chars:?} in MSH-2 (expected 4-5 distinct punctuation characters)"
            ),
            Self::InvalidSegmentId(id) => write!(f, "invalid segment identifier {id:?}"),
        }
    }
}

/// A parse failure with the position at which it was detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The kind of failure.
    pub kind: ParseErrorKind,
    /// 1-based physical line in the input (CR, LF and CRLF each end one line).
    pub line: usize,
    /// 1-based index of the segment within the message (blank lines are not counted).
    pub segment: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, segment {}: {}", self.line, self.segment, self.kind)
    }
}

impl std::error::Error for ParseError {}
