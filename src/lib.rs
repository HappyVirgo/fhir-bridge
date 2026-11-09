//! `fhir-bridge`: an HL7 v2.x parser and HL7 v2 to FHIR R4 converter with optional
//! HIPAA Safe Harbor-style de-identification.
//!
//! ```
//! use fhir_bridge::{fhir, hl7::Message};
//!
//! let text = "MSH|^~\\&|EHR|FAC|||20260101120000||ADT^A04|MSG1|P|2.5.1\r\
//!             PID|1||MRN1^^^HOSP^MR||SAMPLE^ALICE||19800101|F\r\
//!             PV1|1|O";
//! let message = Message::parse(text)?;
//! let (bundle, warnings) = fhir::message_to_bundle(&message, &Default::default(), None)?;
//! assert_eq!(bundle.entry.len(), 2);
//! assert!(warnings.is_empty());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod datetime;
pub mod deid;
pub mod fhir;
pub mod hl7;
pub mod synth;
pub mod validate;

/// Lower-case hexadecimal encoding of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}
