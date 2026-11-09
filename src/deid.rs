//! HIPAA Safe Harbor-style de-identification of converted resources.
//!
//! Direct identifiers (names, street addresses, telecom, SSN) are removed. Medical record and
//! visit numbers are replaced by keyed-hash pseudonyms (HMAC-SHA256 with a secret salt), so the
//! same patient always maps to the same pseudonym without the mapping being reversible by anyone
//! who lacks the salt. Dates are shifted by a per-patient offset derived from the same key, which
//! preserves intervals between a patient's events. Ages over 89 are collapsed into a `90+` category.

use std::fmt;

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::datetime::DateTime;
use crate::fhir::model::{CodeableConcept, Coding, Extension, Identifier, Patient};
use crate::fhir::Conversion;

/// `Identifier.system` used for pseudonymised identifiers.
pub const PSEUDONYM_SYSTEM: &str = "urn:fhir-bridge:pseudonym";
/// Extension URL carrying the generalised age category for patients older than 89.
pub const AGE_CATEGORY_URL: &str = "urn:fhir-bridge:age-category";
/// Largest date shift, in days, in either direction.
pub const MAX_SHIFT_DAYS: i64 = 365;

/// Three-digit ZIP prefixes whose combined population is 20,000 or fewer (HHS guidance based on
/// 2000 Census data). Safe Harbor requires these to be reported as `000`.
const RESTRICTED_ZIP3: [&str; 17] = [
    "036", "059", "063", "102", "203", "556", "692", "790", "821", "823", "830", "831", "878", "879", "884",
    "890", "893",
];

/// Error returned when constructing a [`Deidentifier`] with an empty salt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptySaltError;

impl fmt::Display for EmptySaltError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a non-empty salt is required for de-identification")
    }
}

impl std::error::Error for EmptySaltError {}

/// Applies de-identification using a secret salt.
#[derive(Clone)]
pub struct Deidentifier {
    salt: Vec<u8>,
}

impl fmt::Debug for Deidentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Deidentifier { salt: <redacted> }")
    }
}

impl Deidentifier {
    /// Creates a de-identifier keyed by `salt`.
    ///
    /// # Errors
    /// Returns [`EmptySaltError`] if `salt` is empty.
    pub fn new(salt: &str) -> Result<Self, EmptySaltError> {
        if salt.is_empty() {
            return Err(EmptySaltError);
        }
        Ok(Self { salt: salt.as_bytes().to_vec() })
    }

    fn mac(&self, purpose: &str, value: &str) -> [u8; 32] {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.salt).expect("HMAC accepts keys of any length");
        mac.update(purpose.as_bytes());
        mac.update(&[0]);
        mac.update(value.as_bytes());
        mac.finalize().into_bytes().into()
    }

    /// A stable pseudonym (16 hex characters) for an identifier value.
    pub fn pseudonym(&self, value: &str) -> String {
        crate::hex(&self.mac("id", value)[..8])
    }

    /// The per-patient date shift in days: never zero, within `±MAX_SHIFT_DAYS`.
    pub fn date_shift_days(&self, patient_key: &str) -> i64 {
        let bytes = self.mac("date-shift", patient_key);
        let n = u64::from_be_bytes(bytes[..8].try_into().expect("8 bytes")) % (2 * MAX_SHIFT_DAYS as u64);
        let n = n as i64;
        if n < MAX_SHIFT_DAYS {
            n - MAX_SHIFT_DAYS
        } else {
            n - MAX_SHIFT_DAYS + 1
        }
    }

    /// De-identifies a conversion in place.
    pub fn apply(&self, conversion: &mut Conversion) {
        let key = patient_key(&conversion.patient);
        let shift = self.date_shift_days(&key);
        let reference_date =
            conversion.message_time.or_else(|| conversion.encounter.as_ref()?.period.as_ref()?.start);
        self.patient(&mut conversion.patient, shift, reference_date);

        if let Some(encounter) = &mut conversion.encounter {
            for id in &mut encounter.identifier {
                self.pseudonymise(id);
            }
            encounter.participant.clear();
            encounter.location.clear();
            if let Some(period) = &mut encounter.period {
                period.start = period.start.map(|d| d.shift_days(shift));
                period.end = period.end.map(|d| d.shift_days(shift));
            }
        }
        for obs in &mut conversion.observations {
            obs.effective_date_time = obs.effective_date_time.map(|d| d.shift_days(shift));
            // Free text can contain names, dates or other identifiers; mask rather than scrub.
            if obs.value_string.take().is_some() {
                obs.data_absent_reason = Some(CodeableConcept::from_coding(Coding::new(
                    "http://terminology.hl7.org/CodeSystem/data-absent-reason",
                    "masked",
                )));
            }
        }
        conversion.message_time = conversion.message_time.map(|d| d.shift_days(shift));
    }

    fn patient(&self, patient: &mut Patient, shift: i64, reference_date: Option<DateTime>) {
        patient.identifier.retain(|id| !is_ssn(id));
        for id in &mut patient.identifier {
            self.pseudonymise(id);
        }
        patient.name.clear();
        patient.telecom.clear();
        for address in &mut patient.address {
            address.line.clear();
            address.city = None;
            address.usage = None;
            address.postal_code = address.postal_code.as_deref().map(generalise_zip);
        }
        patient.address.retain(|a| a.state.is_some() || a.postal_code.is_some() || a.country.is_some());

        let over_89 = match (patient.birth_date, reference_date) {
            (Some(birth), Some(at)) => birth.age_at(&at) > 89,
            _ => false,
        };
        if over_89 {
            patient.birth_date = None;
            patient
                .extension
                .push(Extension { url: AGE_CATEGORY_URL.to_owned(), value_string: "90+".to_owned() });
        } else {
            patient.birth_date = patient.birth_date.map(|d| d.shift_days(shift));
        }
    }

    fn pseudonymise(&self, id: &mut Identifier) {
        let source = format!("{}|{}", id.system.as_deref().unwrap_or(""), id.value);
        id.value = self.pseudonym(&source);
        id.system = Some(PSEUDONYM_SYSTEM.to_owned());
    }
}

fn is_ssn(id: &Identifier) -> bool {
    id.system.as_deref() == Some("http://hl7.org/fhir/sid/us-ssn")
        || id.kind.as_ref().is_some_and(|k| k.coding.iter().any(|c| c.code.as_deref() == Some("SS")))
}

/// The key that ties a patient's pseudonym and date shift together: the first non-SSN
/// identifier, or the name and birth date when the message carries no identifier.
fn patient_key(patient: &Patient) -> String {
    if let Some(id) = patient.identifier.iter().find(|id| !is_ssn(id)) {
        return format!("{}|{}", id.system.as_deref().unwrap_or(""), id.value);
    }
    let name = patient.name.first();
    format!(
        "name:{}|{}|{}",
        name.and_then(|n| n.family.as_deref()).unwrap_or(""),
        name.map(|n| n.given.join(" ")).unwrap_or_default(),
        patient.birth_date.map(|d| d.to_string()).unwrap_or_default()
    )
}

/// Keeps the first three digits of a ZIP code, or `000` for sparsely populated prefixes.
fn generalise_zip(zip: &str) -> String {
    let prefix: String = zip.chars().take_while(char::is_ascii_digit).take(3).collect();
    if prefix.len() < 3 || RESTRICTED_ZIP3.contains(&prefix.as_str()) {
        "000".to_owned()
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudonyms_are_stable_and_salt_dependent() {
        let a = Deidentifier::new("salt-a").unwrap();
        let b = Deidentifier::new("salt-b").unwrap();
        assert_eq!(a.pseudonym("MRN1"), a.pseudonym("MRN1"));
        assert_ne!(a.pseudonym("MRN1"), a.pseudonym("MRN2"));
        assert_ne!(a.pseudonym("MRN1"), b.pseudonym("MRN1"));
        assert_eq!(a.pseudonym("MRN1").len(), 16);
        assert!(Deidentifier::new("").is_err());
    }

    #[test]
    fn date_shift_is_bounded_and_never_zero() {
        let d = Deidentifier::new("k").unwrap();
        for i in 0..2000 {
            let shift = d.date_shift_days(&format!("patient-{i}"));
            assert!(shift != 0 && shift.abs() <= MAX_SHIFT_DAYS, "{shift}");
        }
    }

    #[test]
    fn generalises_zip_codes() {
        assert_eq!(generalise_zip("02139-4307"), "021");
        assert_eq!(generalise_zip("03601"), "000");
        assert_eq!(generalise_zip("12"), "000");
    }

    #[test]
    fn debug_output_hides_the_salt() {
        let d = Deidentifier::new("super-secret").unwrap();
        assert!(!format!("{d:?}").contains("super-secret"));
    }
}
