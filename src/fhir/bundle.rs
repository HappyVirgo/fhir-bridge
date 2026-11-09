//! Assembly of converted resources into a FHIR transaction Bundle.

use sha2::{Digest, Sha256};

use super::convert::Conversion;
use super::model::{Bundle, BundleEntry, BundleRequest, Identifier, Reference, Resource};

/// Links the resources of a conversion together and wraps them in a transaction Bundle.
///
/// `fullUrl`s are deterministic UUIDs derived from the message control ID and the entry index,
/// so converting the same message twice yields byte-identical output. The Patient and Encounter
/// are created conditionally (`ifNoneExist` on their first identifier) so that replaying several
/// messages for the same patient does not create duplicates on the server.
pub fn build(conversion: Conversion) -> Bundle {
    let Conversion { patient, encounter, observations, control_id, .. } = conversion;
    let full_url =
        |index: usize| format!("urn:uuid:{}", deterministic_uuid(&format!("{control_id}/{index}")));
    let reference = |url: &str| Reference { reference: Some(url.to_owned()), display: None };

    let patient_url = full_url(0);
    let mut entries =
        vec![entry(&patient_url, "Patient", conditional(&patient.identifier), Resource::Patient(patient))];

    let encounter_url = encounter.map(|mut encounter| {
        let url = full_url(1);
        encounter.subject = reference(&patient_url);
        let condition = conditional(&encounter.identifier);
        entries.push(entry(&url, "Encounter", condition, Resource::Encounter(encounter)));
        url
    });

    for (index, mut observation) in observations.into_iter().enumerate() {
        observation.subject = reference(&patient_url);
        observation.encounter = encounter_url.as_deref().map(reference);
        let url = full_url(index + 2);
        entries.push(entry(&url, "Observation", None, Resource::Observation(Box::new(observation))));
    }

    Bundle { resource_type: "Bundle", kind: "transaction", entry: entries }
}

fn entry(
    full_url: &str,
    resource_type: &str,
    if_none_exist: Option<String>,
    resource: Resource,
) -> BundleEntry {
    BundleEntry {
        full_url: full_url.to_owned(),
        resource,
        request: BundleRequest { method: "POST".to_owned(), url: resource_type.to_owned(), if_none_exist },
    }
}

/// `identifier=system|value` search for the first identifier that has a system.
fn conditional(identifiers: &[Identifier]) -> Option<String> {
    identifiers.iter().find_map(|id| Some(format!("identifier={}|{}", id.system.as_ref()?, id.value)))
}

/// A name-based (version 5 style, SHA-256) UUID string for `seed`.
fn deterministic_uuid(seed: &str) -> String {
    let digest = Sha256::digest(seed.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0f) | 0x50;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex = crate::hex(&b);
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuids_are_deterministic_and_well_formed() {
        let a = deterministic_uuid("MSG1/0");
        assert_eq!(a, deterministic_uuid("MSG1/0"));
        assert_ne!(a, deterministic_uuid("MSG1/1"));
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "5");
    }
}
