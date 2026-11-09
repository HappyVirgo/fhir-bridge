//! Decoding of HL7 v2 escape sequences (`\F\`, `\S\`, `\R\`, `\E\`, `\T\`, `\Xhh\`, `\.br\`).

use std::borrow::Cow;

use super::Delimiters;

/// Decodes the escape sequences in `raw` using the message's delimiters.
///
/// Returns a borrowed slice when the value contains no escape character, so the common case
/// allocates nothing. Unknown sequences are kept verbatim rather than dropped, and an
/// unterminated escape character is treated as a literal.
pub fn decode<'a>(raw: &'a str, delims: &Delimiters) -> Cow<'a, str> {
    let esc = delims.escape as char;
    if !raw.contains(esc) {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find(esc) {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find(esc) else {
            out.push_str(&rest[start..]);
            return Cow::Owned(out);
        };
        let sequence = &after[..end];
        if !push_sequence(&mut out, sequence, delims) {
            out.push(esc);
            out.push_str(sequence);
            out.push(esc);
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Appends the text represented by one escape sequence; returns `false` if it is not recognised.
fn push_sequence(out: &mut String, sequence: &str, delims: &Delimiters) -> bool {
    match sequence {
        "F" => out.push(delims.field as char),
        "S" => out.push(delims.component as char),
        "T" => out.push(delims.subcomponent as char),
        "R" => out.push(delims.repetition as char),
        "E" => out.push(delims.escape as char),
        ".br" => out.push('\n'),
        // Highlighting on/off carries no text.
        "H" | "N" => {}
        _ => match sequence.strip_prefix('X').and_then(decode_hex) {
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(text) => out.push_str(&text),
                // Not UTF-8: interpret as ISO-8859-1, the most common legacy HL7 charset.
                Err(err) => out.extend(err.into_bytes().into_iter().map(char::from)),
            },
            None => return false,
        },
    }
    true
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if hex.is_empty() || hex.len() % 2 == 1 {
        return None;
    }
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d() -> Delimiters {
        Delimiters::default()
    }

    #[test]
    fn plain_text_is_borrowed() {
        assert!(matches!(decode("NO ESCAPES", &d()), Cow::Borrowed(_)));
    }

    #[test]
    fn decodes_all_delimiter_escapes() {
        assert_eq!(decode(r"a\F\b\S\c\R\d\E\e\T\f", &d()), r"a|b^c~d\e&f");
    }

    #[test]
    fn decodes_hex_and_line_breaks() {
        assert_eq!(decode(r"caf\XC3A9\\.br\next", &d()), "café\nnext");
        assert_eq!(decode(r"\XE9\", &d()), "é");
    }

    #[test]
    fn strips_highlighting_and_keeps_unknown_sequences() {
        assert_eq!(decode(r"\H\BOLD\N\ \Zfoo\", &d()), r"BOLD \Zfoo\");
    }

    #[test]
    fn unterminated_escape_is_literal() {
        assert_eq!(decode(r"50\50", &d()), r"50\50");
    }

    #[test]
    fn honours_custom_escape_character() {
        let delims = Delimiters { escape: b'!', ..Delimiters::default() };
        assert_eq!(decode("A!F!B", &delims), "A|B");
    }
}
