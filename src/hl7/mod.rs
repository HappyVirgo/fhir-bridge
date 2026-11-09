//! Zero-copy HL7 v2.x message parser.
//!
//! [`Message::parse`] splits the input into segments and fields once, keeping `&str` slices into
//! the original buffer. Components, repetitions and sub-components are split lazily on access, and
//! escape sequences are only decoded (and only allocate) when [`Field::value`] is called on a value
//! that actually contains the escape character.

mod error;
pub mod escape;

use std::borrow::Cow;

pub use error::{ParseError, ParseErrorKind};

/// The five HL7 delimiters declared in `MSH-1` and `MSH-2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delimiters {
    /// Field separator (`MSH-1`), normally `|`.
    pub field: u8,
    /// Component separator, normally `^`.
    pub component: u8,
    /// Repetition separator, normally `~`.
    pub repetition: u8,
    /// Escape character, normally `\`.
    pub escape: u8,
    /// Sub-component separator, normally `&`.
    pub subcomponent: u8,
}

impl Default for Delimiters {
    fn default() -> Self {
        Self { field: b'|', component: b'^', repetition: b'~', escape: b'\\', subcomponent: b'&' }
    }
}

impl Delimiters {
    /// Reads the delimiters from the start of an `MSH` line.
    fn from_msh(line: &str) -> Result<Self, ParseErrorKind> {
        let bytes = line.as_bytes();
        if bytes.len() < 8 {
            return Err(ParseErrorKind::TruncatedMsh);
        }
        let field = bytes[3];
        let encoding = &bytes[4..];
        let encoding = &encoding[..encoding.iter().position(|&b| b == field).unwrap_or(encoding.len())];
        let mut all = vec![field];
        all.extend_from_slice(encoding);
        let valid_char = |b: &u8| b.is_ascii_punctuation();
        let distinct = all.iter().enumerate().all(|(i, b)| !all[..i].contains(b));
        // Four encoding characters; HL7 v2.7+ adds a fifth (truncation), which is accepted and ignored.
        if !(4..=5).contains(&encoding.len()) || !all.iter().all(valid_char) || !distinct {
            return Err(ParseErrorKind::InvalidEncodingCharacters(
                String::from_utf8_lossy(&all).into_owned(),
            ));
        }
        Ok(Self {
            field,
            component: encoding[0],
            repetition: encoding[1],
            escape: encoding[2],
            subcomponent: encoding[3],
        })
    }
}

/// A parsed HL7 v2 message borrowing from its input.
#[derive(Debug, Clone)]
pub struct Message<'a> {
    delimiters: Delimiters,
    segments: Vec<Segment<'a>>,
}

/// One segment (line) of a message.
#[derive(Debug, Clone)]
pub struct Segment<'a> {
    /// Fields indexed by HL7 field number; index 0 is the segment identifier.
    fields: Vec<&'a str>,
    line: usize,
    delimiters: Delimiters,
}

/// A field, repetition, component or sub-component value. All levels share this type: splitting a
/// value by a separator it does not contain simply yields the value itself.
#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
    raw: &'a str,
    delimiters: Delimiters,
    /// `MSH-1` and `MSH-2` hold the delimiters themselves and must never be split or decoded.
    literal: bool,
}

impl<'a> Message<'a> {
    /// Parses one HL7 v2 message. Segments may be terminated by CR, LF or CRLF; blank lines and a
    /// leading UTF-8 byte-order mark are ignored.
    ///
    /// # Errors
    /// Returns a [`ParseError`] with the line and segment position if the message does not start
    /// with a valid `MSH` segment or contains a malformed segment identifier.
    pub fn parse(input: &'a str) -> Result<Self, ParseError> {
        let input = input.strip_prefix('\u{feff}').unwrap_or(input);
        let mut lines = lines(input).filter(|(_, text)| !text.trim().is_empty());
        let Some((first_line, msh)) = lines.next() else {
            return Err(ParseError { kind: ParseErrorKind::Empty, line: 1, segment: 1 });
        };
        let at = |kind, line, segment| ParseError { kind, line, segment };
        if !msh.starts_with("MSH") {
            let found = msh.chars().take(3).collect();
            return Err(at(ParseErrorKind::MissingMsh { found }, first_line, 1));
        }
        let delimiters = Delimiters::from_msh(msh).map_err(|kind| at(kind, first_line, 1))?;

        let mut segments = vec![Segment::new_msh(msh, first_line, delimiters)];
        for (index, (line, text)) in lines.enumerate() {
            let segment = Segment::new(text, line, delimiters);
            if !is_valid_segment_id(segment.id()) {
                let id = segment.id().to_owned();
                return Err(at(ParseErrorKind::InvalidSegmentId(id), line, index + 2));
            }
            segments.push(segment);
        }
        Ok(Self { delimiters, segments })
    }

    /// The delimiters declared by this message's `MSH` segment.
    pub fn delimiters(&self) -> Delimiters {
        self.delimiters
    }

    /// All segments in order, starting with `MSH`.
    pub fn segments(&self) -> &[Segment<'a>] {
        &self.segments
    }

    /// The first segment with the given identifier.
    pub fn segment(&self, id: &str) -> Option<&Segment<'a>> {
        self.segments.iter().find(|s| s.id() == id)
    }

    /// The `MSH` segment (always present in a parsed message).
    pub fn msh(&self) -> &Segment<'a> {
        &self.segments[0]
    }

    /// Message code and trigger event from `MSH-9`, e.g. `("ADT", "A01")`.
    pub fn message_type(&self) -> (&'a str, &'a str) {
        let field = self.msh().field(9);
        (field.component(1).raw(), field.component(2).raw())
    }

    /// Message control ID (`MSH-10`).
    pub fn control_id(&self) -> &'a str {
        self.msh().field(10).raw()
    }
}

impl<'a> Segment<'a> {
    fn new(text: &'a str, line: usize, delimiters: Delimiters) -> Self {
        Self { fields: text.split(delimiters.field as char).collect(), line, delimiters }
    }

    fn new_msh(text: &'a str, line: usize, delimiters: Delimiters) -> Self {
        let mut segment = Self::new(text, line, delimiters);
        // Splitting "MSH|^~\&|..." yields ["MSH", "^~\&", ...]; insert MSH-1 (the separator itself).
        segment.fields.insert(1, &text[3..4]);
        segment
    }

    /// The three-character segment identifier, e.g. `PID`.
    pub fn id(&self) -> &'a str {
        self.fields[0]
    }

    /// The 1-based physical line on which this segment starts.
    pub fn line(&self) -> usize {
        self.line
    }

    /// Field `n` (1-based, HL7 numbering). Missing fields are returned as empty values.
    pub fn field(&self, n: usize) -> Field<'a> {
        Field {
            raw: self.fields.get(n).copied().unwrap_or(""),
            delimiters: self.delimiters,
            literal: self.id() == "MSH" && n <= 2,
        }
    }
}

impl<'a> Field<'a> {
    /// The undecoded value exactly as it appears in the message.
    pub fn raw(&self) -> &'a str {
        self.raw
    }

    /// Whether the value is empty. Note that the HL7 explicit null `""` is not empty.
    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    /// The value with escape sequences decoded; borrows when nothing needs decoding.
    pub fn value(&self) -> Cow<'a, str> {
        if self.literal {
            Cow::Borrowed(self.raw)
        } else {
            escape::decode(self.raw, &self.delimiters)
        }
    }

    /// The value decoded and converted to an owned string, or `None` if empty.
    pub fn non_empty(&self) -> Option<String> {
        let value = self.value();
        (!value.is_empty()).then(|| value.into_owned())
    }

    /// Iterates over the repetitions of this field (a single item when it does not repeat).
    pub fn repetitions(&self) -> impl Iterator<Item = Field<'a>> + 'a {
        let this = *self;
        let separator = if self.literal { None } else { Some(self.delimiters.repetition as char) };
        let parts: Box<dyn Iterator<Item = &'a str>> = match separator {
            Some(sep) if !self.raw.is_empty() => Box::new(self.raw.split(sep)),
            _ => Box::new(std::iter::once(self.raw)),
        };
        parts.map(move |raw| this.with_raw(raw))
    }

    /// Component `n` (1-based) of the first repetition.
    pub fn component(&self, n: usize) -> Field<'a> {
        let first = self.part(self.delimiters.repetition, 1);
        first.part(self.delimiters.component, n)
    }

    /// Sub-component `n` (1-based) of this value.
    pub fn subcomponent(&self, n: usize) -> Field<'a> {
        self.part(self.delimiters.subcomponent, n)
    }

    fn part(&self, separator: u8, n: usize) -> Field<'a> {
        if self.literal {
            return if n == 1 { *self } else { self.with_raw("") };
        }
        let raw = self.raw.split(separator as char).nth(n.saturating_sub(1)).unwrap_or("");
        self.with_raw(raw)
    }

    fn with_raw(&self, raw: &'a str) -> Field<'a> {
        Field { raw, ..*self }
    }
}

/// Splits a buffer holding several messages (for example a batch file) into one slice per
/// message, starting a new message at every `MSH` line. `FHS`, `BHS`, `BTS` and `FTS` batch
/// envelope segments and blank lines between messages are skipped.
pub fn split_messages(input: &str) -> Vec<&str> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut messages = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    for (_, text) in lines(input) {
        let offset = text.as_ptr() as usize - input.as_ptr() as usize;
        let is_envelope = ["FHS", "BHS", "BTS", "FTS"].iter().any(|id| text.starts_with(id));
        if text.starts_with("MSH") || is_envelope {
            if let Some(s) = start.take() {
                messages.push(&input[s..end]);
            }
        }
        // Content that does not start with MSH still becomes a message, so that parsing it
        // reports a positioned error instead of the data being silently dropped.
        if start.is_none() && !is_envelope && !text.trim().is_empty() {
            start = Some(offset);
        }
        end = offset + text.len();
    }
    if let Some(s) = start {
        messages.push(&input[s..end]);
    }
    messages
}

/// Iterates over `(line_number, text)` pairs, treating CR, LF and CRLF as line terminators.
fn lines(input: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut rest = input;
    let mut line = 0;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        line += 1;
        let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
        let text = &rest[..end];
        let skip = if rest[end..].starts_with("\r\n") { 2 } else { usize::from(end < rest.len()) };
        rest = &rest[end + skip..];
        Some((line, text))
    })
}

fn is_valid_segment_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 3
        && bytes[0].is_ascii_uppercase()
        && bytes[1..].iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADT: &str = "MSH|^~\\&|SENDER|FAC|RECV|FAC|20260101120000||ADT^A01^ADT_A01|MSG001|P|2.5.1\r\
                       PID|1||MRN123^^^HOSP^MR~SSN9^^^USSSA^SS||SAMPLE^ALICE^Q||19800101|F\r\
                       PV1|1|I|4E^401^A";

    #[test]
    fn parses_segments_and_fields() {
        let msg = Message::parse(ADT).unwrap();
        assert_eq!(msg.segments().len(), 3);
        assert_eq!(msg.message_type(), ("ADT", "A01"));
        assert_eq!(msg.control_id(), "MSG001");
        let pid = msg.segment("PID").unwrap();
        assert_eq!(pid.line(), 2);
        assert_eq!(pid.field(5).component(2).raw(), "ALICE");
        assert_eq!(pid.field(99).raw(), "");
    }

    #[test]
    fn msh_numbering_includes_the_field_separator() {
        let msg = Message::parse(ADT).unwrap();
        assert_eq!(msg.msh().field(1).raw(), "|");
        assert_eq!(msg.msh().field(2).value(), "^~\\&");
        assert_eq!(msg.msh().field(2).component(2).raw(), "");
        assert_eq!(msg.msh().field(3).raw(), "SENDER");
    }

    #[test]
    fn splits_repetitions_and_subcomponents() {
        let msg = Message::parse(ADT).unwrap();
        let ids: Vec<_> = msg
            .segment("PID")
            .unwrap()
            .field(3)
            .repetitions()
            .map(|r| (r.component(1).raw(), r.component(5).raw()))
            .collect();
        assert_eq!(ids, [("MRN123", "MR"), ("SSN9", "SS")]);
        let msg = Message::parse("MSH|^~\\&|A\rPID|1||X^^^HOSP&1.2.3&ISO").unwrap();
        let hd = msg.segment("PID").unwrap().field(3).component(4);
        assert_eq!(hd.subcomponent(2).raw(), "1.2.3");
    }

    #[test]
    fn accepts_lf_crlf_and_custom_delimiters() {
        let lf = ADT.replace('\r', "\n");
        let crlf = ADT.replace('\r', "\r\n\r\n");
        for text in [lf.as_str(), crlf.as_str()] {
            assert_eq!(Message::parse(text).unwrap().segments().len(), 3);
        }
        let custom = Message::parse("MSH#$*!@#APP\nPID#1##ID1$$$AUTH*ID2").unwrap();
        let pid3 = custom.segment("PID").unwrap().field(3);
        assert_eq!(pid3.component(4).raw(), "AUTH");
        assert_eq!(pid3.repetitions().count(), 2);
    }

    #[test]
    fn reports_error_positions() {
        let err = Message::parse("\n\nPID|1").unwrap_err();
        assert_eq!(err.line, 3);
        assert!(matches!(err.kind, ParseErrorKind::MissingMsh { .. }));

        let err = Message::parse("MSH|^~\\&|A\rPID|1\r\rp1d|2").unwrap_err();
        assert_eq!((err.line, err.segment), (4, 3));
        assert_eq!(err.kind, ParseErrorKind::InvalidSegmentId("p1d".into()));

        let err = Message::parse("MSH|^^\\&|A").unwrap_err();
        assert!(matches!(err.kind, ParseErrorKind::InvalidEncodingCharacters(_)));
        assert_eq!(Message::parse("MSH|^~").unwrap_err().kind, ParseErrorKind::TruncatedMsh);
        assert_eq!(Message::parse(" \r\n").unwrap_err().kind, ParseErrorKind::Empty);
    }

    #[test]
    fn splits_batch_files() {
        let batch = format!("FHS|^~\\&\rBHS|^~\\&\r{ADT}\r{ADT}\nBTS|2\rFTS|1");
        let messages = split_messages(&batch);
        assert_eq!(messages.len(), 2);
        assert!(messages.iter().all(|m| m.starts_with("MSH") && m.ends_with("4E^401^A")));
        assert_eq!(split_messages("\r\nPID|1\rMSH|^~\\&|A"), ["PID|1", "MSH|^~\\&|A"]);
        assert!(split_messages(" \n").is_empty());
    }
}
