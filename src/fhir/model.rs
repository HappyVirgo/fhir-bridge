//! The subset of FHIR R4 data types and resources produced by the converter.
//!
//! Only serialization is needed; every optional element is skipped when absent so the output is
//! minimal valid FHIR JSON.

use serde::Serialize;

use crate::datetime::DateTime;

/// FHIR `Coding`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Coding {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

impl Coding {
    /// A coding with a system and code but no display text.
    pub fn new(system: &str, code: &str) -> Self {
        Self { system: Some(system.to_owned()), code: Some(code.to_owned()), display: None }
    }
}

/// FHIR `CodeableConcept`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CodeableConcept {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub coding: Vec<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl CodeableConcept {
    /// A concept with exactly one coding.
    pub fn from_coding(coding: Coding) -> Self {
        Self { coding: vec![coding], text: None }
    }
}

/// FHIR `Identifier`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Identifier {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub value: String,
}

/// FHIR `HumanName`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct HumanName {
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub given: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prefix: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub suffix: Vec<String>,
}

/// FHIR `Address`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub line: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postal_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
}

/// FHIR `ContactPoint`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ContactPoint {
    pub system: String,
    pub value: String,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

/// FHIR `Reference`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Reference {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
}

/// FHIR `Period`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Period {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<DateTime>,
}

/// FHIR `Quantity`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Quantity {
    pub value: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

/// FHIR `Extension` (string-valued only).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub url: String,
    pub value_string: String,
}

/// FHIR R4 `Patient`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Patient {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extension: Vec<Extension>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub identifier: Vec<Identifier>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub name: Vec<HumanName>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub telecom: Vec<ContactPoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gender: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub birth_date: Option<DateTime>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub address: Vec<Address>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marital_status: Option<CodeableConcept>,
}

/// A participant of an `Encounter`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EncounterParticipant {
    #[serde(rename = "type")]
    pub kind: Vec<CodeableConcept>,
    pub individual: Reference,
}

/// A location of an `Encounter`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct EncounterLocation {
    pub location: Reference,
}

/// FHIR R4 `Encounter`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Encounter {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub identifier: Vec<Identifier>,
    pub status: String,
    pub class: Coding,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub participant: Vec<EncounterParticipant>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<Period>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub location: Vec<EncounterLocation>,
}

/// One `Observation.referenceRange` entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReferenceRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub high: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// FHIR R4 `Observation`. At most one of the `value_*` elements and `data_absent_reason` is set.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Observation {
    pub status: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub category: Vec<CodeableConcept>,
    pub code: CodeableConcept,
    pub subject: Reference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encounter: Option<Reference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_date_time: Option<DateTime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_quantity: Option<Quantity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_codeable_concept: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_string: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_absent_reason: Option<CodeableConcept>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub interpretation: Vec<CodeableConcept>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reference_range: Vec<ReferenceRange>,
}

/// Any resource that can appear in the output bundle.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "resourceType")]
pub enum Resource {
    Patient(Patient),
    Encounter(Encounter),
    Observation(Box<Observation>),
}

/// The `request` of a transaction bundle entry.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRequest {
    pub method: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub if_none_exist: Option<String>,
}

/// One entry of a transaction bundle.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleEntry {
    pub full_url: String,
    pub resource: Resource,
    pub request: BundleRequest,
}

/// FHIR R4 transaction `Bundle`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bundle {
    pub resource_type: &'static str,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub entry: Vec<BundleEntry>,
}
