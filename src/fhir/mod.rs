//! HL7 v2 to FHIR R4 conversion.

pub mod bundle;
pub mod convert;
pub mod model;

pub use convert::{convert, Conversion, ConvertError, ConvertOptions};
pub use model::Bundle;

use crate::deid::Deidentifier;
use crate::hl7::Message;

/// Converts a parsed message to a transaction Bundle, optionally de-identifying it first.
/// Returns the bundle together with any non-fatal conversion warnings.
///
/// # Errors
/// See [`convert()`].
pub fn message_to_bundle(
    message: &Message<'_>,
    options: &ConvertOptions,
    deidentifier: Option<&Deidentifier>,
) -> Result<(Bundle, Vec<String>), ConvertError> {
    let mut conversion = convert(message, options)?;
    if let Some(deid) = deidentifier {
        deid.apply(&mut conversion);
    }
    let warnings = std::mem::take(&mut conversion.warnings);
    Ok((bundle::build(conversion), warnings))
}
