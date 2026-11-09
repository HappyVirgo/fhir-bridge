//! Mapping of HL7 v2 ADT^A01/A04/A08 and ORU^R01 segments to FHIR R4 resources.

use std::fmt;

use super::model::*;
use crate::datetime::DateTime;
use crate::hl7::{Field, Message, Segment};

const LOINC: &str = "http://loinc.org";
const SNOMED: &str = "http://snomed.info/sct";
const UCUM: &str = "http://unitsofmeasure.org";
const US_SSN: &str = "http://hl7.org/fhir/sid/us-ssn";
const V2_0203: &str = "http://terminology.hl7.org/CodeSystem/v2-0203";
const V2_0004: &str = "http://terminology.hl7.org/CodeSystem/v2-0004";
const V3_ACT_CODE: &str = "http://terminology.hl7.org/CodeSystem/v3-ActCode";
const V3_MARITAL: &str = "http://terminology.hl7.org/CodeSystem/v3-MaritalStatus";
const V3_INTERPRETATION: &str = "http://terminology.hl7.org/CodeSystem/v3-ObservationInterpretation";
const V3_PARTICIPATION: &str = "http://terminology.hl7.org/CodeSystem/v3-ParticipationType";
const OBS_CATEGORY: &str = "http://terminology.hl7.org/CodeSystem/observation-category";

/// Settings that influence conversion.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// UTC offset (minutes) assumed for HL7 timestamps that carry a time but no offset.
    pub default_offset_minutes: i16,
    /// Prefix for `Identifier.system` when the assigning authority has only a namespace ID.
    pub identifier_system_prefix: String,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self { default_offset_minutes: 0, identifier_system_prefix: "urn:fhir-bridge:assigner:".to_owned() }
    }
}

/// Resources extracted from one message, before bundling. References between them are filled in
/// by [`super::bundle::build`].
#[derive(Debug, Clone)]
pub struct Conversion {
    pub patient: Patient,
    pub encounter: Option<Encounter>,
    pub observations: Vec<Observation>,
    /// `MSH-7`, used as the reference date for age calculations.
    pub message_time: Option<DateTime>,
    /// `MSH-10`, used to derive deterministic `fullUrl`s.
    pub control_id: String,
    /// Non-fatal problems such as unparseable dates or unknown codes.
    pub warnings: Vec<String>,
}

/// Reasons a parsed message cannot be converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// The message type is not one of ADT^A01, ADT^A04, ADT^A08 or ORU^R01.
    UnsupportedMessageType(String),
    /// A segment required for conversion is absent.
    MissingSegment(&'static str),
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedMessageType(t) => {
                write!(f, "unsupported message type {t:?} (supported: ADT^A01, ADT^A04, ADT^A08, ORU^R01)")
            }
            Self::MissingSegment(id) => write!(f, "required segment {id} is missing"),
        }
    }
}

impl std::error::Error for ConvertError {}

/// Whether `MSH-9` names a message type this converter supports.
pub fn is_supported(code: &str, event: &str) -> bool {
    matches!((code, event), ("ADT", "A01" | "A04" | "A08") | ("ORU", "R01"))
}

/// Converts a parsed message into FHIR resources.
///
/// # Errors
/// Fails for unsupported message types and when the `PID` segment is missing. Problems with
/// individual values are reported in [`Conversion::warnings`] instead.
pub fn convert(msg: &Message<'_>, opts: &ConvertOptions) -> Result<Conversion, ConvertError> {
    let (code, event) = msg.message_type();
    if !is_supported(code, event) {
        return Err(ConvertError::UnsupportedMessageType(format!("{code}^{event}")));
    }
    let pid = msg.segment("PID").ok_or(ConvertError::MissingSegment("PID"))?;
    let mut cx = Converter { opts, warnings: Vec::new() };
    let patient = cx.patient(pid);
    let encounter = msg.segment("PV1").map(|pv1| cx.encounter(pv1));
    let mut observations = Vec::new();
    let mut order_time = None;
    for segment in msg.segments() {
        match segment.id() {
            "OBR" => order_time = cx.datetime(segment, 7),
            "OBX" => observations.push(cx.observation(segment, order_time, code == "ORU")),
            _ => {}
        }
    }
    let message_time = cx.datetime(msg.msh(), 7);
    Ok(Conversion {
        patient,
        encounter,
        observations,
        message_time,
        control_id: msg.control_id().to_owned(),
        warnings: cx.warnings,
    })
}

struct Converter<'o> {
    opts: &'o ConvertOptions,
    warnings: Vec<String>,
}

impl Converter<'_> {
    fn warn(&mut self, segment: &Segment<'_>, field: usize, message: String) {
        self.warnings.push(format!("{}-{field} (line {}): {message}", segment.id(), segment.line()));
    }

    fn datetime(&mut self, segment: &Segment<'_>, n: usize) -> Option<DateTime> {
        let raw = segment.field(n).component(1).raw();
        if raw.is_empty() {
            return None;
        }
        let parsed = DateTime::parse_hl7(raw);
        if parsed.is_none() {
            self.warn(segment, n, format!("invalid timestamp {raw:?} ignored"));
        }
        parsed.map(|dt| dt.with_default_offset(self.opts.default_offset_minutes))
    }

    fn patient(&mut self, pid: &Segment<'_>) -> Patient {
        let mut identifier: Vec<_> =
            pid.field(3).repetitions().filter_map(|cx| self.identifier(cx)).collect();
        if let Some(ssn) = pid.field(19).non_empty() {
            identifier.push(Identifier {
                kind: Some(v2_identifier_type("SS")),
                system: Some(US_SSN.to_owned()),
                value: ssn,
            });
        }
        let gender = match pid.field(8).raw() {
            "" => None,
            "M" => Some("male"),
            "F" => Some("female"),
            "O" | "A" | "N" => Some("other"),
            "U" => Some("unknown"),
            other => {
                self.warn(pid, 8, format!("unknown administrative sex {other:?}"));
                Some("unknown")
            }
        };
        let telecom = pid.field(13).repetitions().filter_map(|t| contact(t, "home"));
        let business = pid.field(14).repetitions().filter_map(|t| contact(t, "work"));
        Patient {
            extension: Vec::new(),
            identifier,
            name: pid.field(5).repetitions().filter_map(human_name).collect(),
            telecom: telecom.chain(business).collect(),
            gender: gender.map(str::to_owned),
            birth_date: self.datetime(pid, 7).map(DateTime::to_date),
            address: pid.field(11).repetitions().filter_map(address).collect(),
            marital_status: self.marital_status(pid),
        }
    }

    fn identifier(&self, cx: Field<'_>) -> Option<Identifier> {
        let value = cx.component(1).non_empty()?;
        let authority = cx.component(4);
        let system = match (authority.subcomponent(2).raw(), authority.subcomponent(3).raw()) {
            (oid, "ISO") if !oid.is_empty() => Some(format!("urn:oid:{oid}")),
            _ => authority
                .subcomponent(1)
                .non_empty()
                .map(|ns| format!("{}{ns}", self.opts.identifier_system_prefix)),
        };
        Some(Identifier { kind: cx.component(5).non_empty().map(|t| v2_identifier_type(&t)), system, value })
    }

    fn marital_status(&mut self, pid: &Segment<'_>) -> Option<CodeableConcept> {
        let code = match pid.field(16).component(1).raw() {
            "" => return None,
            c @ ("M" | "S" | "D" | "W" | "I" | "U") => c,
            "A" => "L",
            "P" => "T",
            other => {
                self.warn(pid, 16, format!("unknown marital status {other:?}"));
                return None;
            }
        };
        Some(CodeableConcept::from_coding(Coding::new(V3_MARITAL, code)))
    }

    fn encounter(&mut self, pv1: &Segment<'_>) -> Encounter {
        let class = match pv1.field(2).raw() {
            "E" => Coding::new(V3_ACT_CODE, "EMER"),
            "I" => Coding::new(V3_ACT_CODE, "IMP"),
            "O" => Coding::new(V3_ACT_CODE, "AMB"),
            "P" => Coding::new(V3_ACT_CODE, "PRENC"),
            "" => Coding::new(V3_ACT_CODE, "AMB"),
            other => Coding::new(V2_0004, other),
        };
        let mut participant = Vec::new();
        for (field, role) in [(7, "ATND"), (17, "ADM")] {
            participant.extend(pv1.field(field).repetitions().filter_map(|xcn| {
                let name = [xcn.component(6), xcn.component(3), xcn.component(2)]
                    .iter()
                    .filter_map(Field::non_empty)
                    .collect::<Vec<_>>()
                    .join(" ");
                (!name.is_empty()).then(|| EncounterParticipant {
                    kind: vec![CodeableConcept::from_coding(Coding::new(V3_PARTICIPATION, role))],
                    individual: Reference { reference: None, display: Some(name) },
                })
            }));
        }
        let pl = pv1.field(3);
        let location_name = (1..=3).filter_map(|n| pl.component(n).non_empty()).collect::<Vec<_>>().join(" ");
        let start = self.datetime(pv1, 44);
        let end = self.datetime(pv1, 45);
        Encounter {
            identifier: self
                .identifier(pv1.field(19))
                .map(|mut id| {
                    id.kind.get_or_insert_with(|| v2_identifier_type("VN"));
                    id
                })
                .into_iter()
                .collect(),
            status: if end.is_some() { "finished" } else { "in-progress" }.to_owned(),
            class,
            subject: Reference::default(),
            participant,
            period: (start.is_some() || end.is_some()).then_some(Period { start, end }),
            location: (!location_name.is_empty())
                .then(|| EncounterLocation {
                    location: Reference { reference: None, display: Some(location_name) },
                })
                .into_iter()
                .collect(),
        }
    }

    fn observation(&mut self, obx: &Segment<'_>, order_time: Option<DateTime>, lab: bool) -> Observation {
        let code = coded(obx.field(3)).unwrap_or_else(|| {
            self.warn(obx, 3, "missing observation identifier".to_owned());
            CodeableConcept { coding: Vec::new(), text: Some("Unknown observation".to_owned()) }
        });
        let status = match obx.field(11).raw() {
            "F" => "final",
            "C" => "corrected",
            "P" | "R" | "S" => "preliminary",
            "I" => "registered",
            "X" => "cancelled",
            "D" | "W" => "entered-in-error",
            "" => "unknown",
            other => {
                self.warn(obx, 11, format!("unknown result status {other:?}"));
                "unknown"
            }
        };
        let units = obx.field(6);
        let mut obs = Observation {
            status: status.to_owned(),
            category: if lab {
                vec![CodeableConcept::from_coding(Coding::new(OBS_CATEGORY, "laboratory"))]
            } else {
                Vec::new()
            },
            code,
            effective_date_time: self.datetime(obx, 14).or(order_time),
            reference_range: reference_range(obx.field(7).value().trim(), units).into_iter().collect(),
            ..Observation::default()
        };
        for flag in obx.field(8).repetitions().map(|f| f.raw()).filter(|f| !f.is_empty()) {
            if ["H", "L", "HH", "LL", "N", "A", "AA", "S", "R", "I", "POS", "NEG"].contains(&flag) {
                obs.interpretation.push(CodeableConcept::from_coding(Coding::new(V3_INTERPRETATION, flag)));
            } else {
                self.warn(obx, 8, format!("unknown abnormal flag {flag:?}"));
            }
        }
        let value = obx.field(5);
        if value.is_empty() {
            return obs;
        }
        match obx.field(2).raw() {
            "NM" => match value.value().trim().parse::<f64>() {
                Ok(number) => obs.value_quantity = Some(quantity(number, units)),
                Err(_) => {
                    self.warn(obx, 5, format!("non-numeric NM value {:?} kept as string", value.raw()));
                    obs.value_string = Some(value.value().into_owned());
                }
            },
            "SN" => structured_numeric(&mut obs, value, units),
            "CE" | "CWE" | "CNE" => obs.value_codeable_concept = coded(value),
            _ => {
                let text: Vec<_> = value.repetitions().map(|r| r.value().into_owned()).collect();
                obs.value_string = Some(text.join("\n"));
            }
        }
        obs
    }
}

fn v2_identifier_type(code: &str) -> CodeableConcept {
    CodeableConcept::from_coding(Coding::new(V2_0203, code))
}

fn system_uri(hl7_system: &str) -> Option<String> {
    match hl7_system {
        "LN" | "LOINC" => Some(LOINC),
        "SCT" | "SNM" | "SNOMED" => Some(SNOMED),
        "UCUM" => Some(UCUM),
        _ => None,
    }
    .map(str::to_owned)
}

/// Converts a CE/CWE value (`code^text^system^altCode^altText^altSystem`) to a CodeableConcept.
fn coded(field: Field<'_>) -> Option<CodeableConcept> {
    let coding: Vec<_> = [(1, 2, 3), (4, 5, 6)]
        .into_iter()
        .filter_map(|(code, display, system)| {
            Some(Coding {
                code: Some(field.component(code).non_empty()?),
                display: field.component(display).non_empty(),
                system: system_uri(field.component(system).raw()),
            })
        })
        .collect();
    let text = field.component(2).non_empty();
    (!coding.is_empty() || text.is_some()).then_some(CodeableConcept { coding, text })
}

/// Builds a Quantity from a number and an OBX-6 units value (`code^text^system`).
fn quantity(value: f64, units: Field<'_>) -> Quantity {
    let code = units.component(1).non_empty();
    let system = system_uri(units.component(3).raw());
    Quantity {
        value,
        comparator: None,
        unit: units.component(2).non_empty().or_else(|| code.clone()),
        code: system.as_ref().and(code),
        system,
    }
}

/// Parses OBX-7 reference ranges such as `3.5-5.0`, `<200` or `>60`; anything else is kept as text.
fn reference_range(raw: &str, units: Field<'_>) -> Option<ReferenceRange> {
    if raw.is_empty() {
        return None;
    }
    let num = |s: &str| s.trim().parse::<f64>().ok().map(|v| quantity(v, units));
    let (low, high) = if let Some(rest) = raw.strip_prefix(['<', '≤']) {
        (None, num(rest.trim_start_matches('=')))
    } else if let Some(rest) = raw.strip_prefix(['>', '≥']) {
        (num(rest.trim_start_matches('=')), None)
    } else {
        // Search from index 1 so a leading minus sign is not taken as the separator.
        match raw.get(1..).and_then(|s| s.find('-')).map(|i| i + 1) {
            Some(i) => (num(&raw[..i]), num(&raw[i + 1..])),
            None => (None, None),
        }
    };
    Some(if low.is_some() || high.is_some() {
        ReferenceRange { low, high, text: None }
    } else {
        ReferenceRange { low: None, high: None, text: Some(raw.to_owned()) }
    })
}

/// Maps an SN value (`comparator^num1^separator^num2`) to a Quantity when it is a single number.
fn structured_numeric(obs: &mut Observation, value: Field<'_>, units: Field<'_>) {
    let comparator = value.component(1).raw();
    let number = value.component(2).raw().trim().parse::<f64>();
    match number {
        Ok(n) if value.component(3).is_empty() && ["", "=", "<", ">", "<=", ">="].contains(&comparator) => {
            let mut q = quantity(n, units);
            q.comparator = (!comparator.is_empty() && comparator != "=").then(|| comparator.to_owned());
            obs.value_quantity = Some(q);
        }
        _ => {
            let parts: String = (1..=4).map(|i| value.component(i).value().into_owned()).collect();
            obs.value_string = Some(parts);
        }
    }
}

/// Converts an XPN repetition (`family^given^middle^suffix^prefix^degree^type`).
fn human_name(xpn: Field<'_>) -> Option<HumanName> {
    let name = HumanName {
        usage: match xpn.component(7).raw() {
            "L" => Some("official"),
            "D" => Some("usual"),
            "M" => Some("maiden"),
            "N" => Some("nickname"),
            _ => None,
        }
        .map(str::to_owned),
        family: xpn.component(1).subcomponent(1).non_empty(),
        given: [2, 3].iter().filter_map(|&n| xpn.component(n).non_empty()).collect(),
        prefix: xpn.component(5).non_empty().into_iter().collect(),
        suffix: xpn.component(4).non_empty().into_iter().collect(),
    };
    (name.family.is_some() || !name.given.is_empty()).then_some(name)
}

/// Converts an XAD repetition (`street^other^city^state^zip^country^type`).
fn address(xad: Field<'_>) -> Option<Address> {
    let address = Address {
        usage: match xad.component(7).raw() {
            "H" => Some("home"),
            "B" => Some("work"),
            "C" => Some("temp"),
            _ => None,
        }
        .map(str::to_owned),
        line: [xad.component(1).subcomponent(1), xad.component(2)]
            .iter()
            .filter_map(Field::non_empty)
            .collect(),
        city: xad.component(3).non_empty(),
        state: xad.component(4).non_empty(),
        postal_code: xad.component(5).non_empty(),
        country: xad.component(6).non_empty(),
    };
    let has_content = !address.line.is_empty()
        || address.city.is_some()
        || address.state.is_some()
        || address.postal_code.is_some()
        || address.country.is_some();
    has_content.then_some(address)
}

/// Converts an XTN repetition (`number^use^equipment^email^country^area^local`).
fn contact(xtn: Field<'_>, default_use: &str) -> Option<ContactPoint> {
    let (use_code, equipment) = (xtn.component(2).raw(), xtn.component(3).raw());
    let email = use_code == "NET" || equipment == "Internet";
    let value = if email {
        xtn.component(4).non_empty().or_else(|| xtn.component(1).non_empty())
    } else {
        xtn.component(1).non_empty().or_else(|| {
            let local = xtn.component(7).non_empty()?;
            Some(match xtn.component(6).non_empty() {
                Some(area) => format!("({area}) {local}"),
                None => local,
            })
        })
    }?;
    let system = match equipment {
        _ if email => "email",
        "FX" => "fax",
        "BP" => "pager",
        _ => "phone",
    };
    let usage = match (use_code, equipment) {
        (_, "CP") => "mobile",
        ("WPN", _) => "work",
        ("PRN" | "ORN" | "VHN", _) => "home",
        _ => default_use,
    };
    Some(ContactPoint { system: system.to_owned(), value, usage: Some(usage.to_owned()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obx(segment: &str) -> Observation {
        let text = format!("MSH|^~\\&|LAB|F|||20260301||ORU^R01|1|P|2.5.1\rPID|1||X1\r{segment}");
        let msg = Message::parse(&text).unwrap();
        convert(&msg, &ConvertOptions::default()).unwrap().observations.remove(0)
    }

    #[test]
    fn maps_numeric_obx_with_loinc_ucum_and_range() {
        let o = obx("OBX|1|NM|2345-7^Glucose^LN||105|mg/dL^mg/dL^UCUM|70-99|H|||F|||202603010800");
        assert_eq!(o.code.coding[0].system.as_deref(), Some(LOINC));
        let q = o.value_quantity.unwrap();
        assert_eq!((q.value, q.code.as_deref(), q.system.as_deref()), (105.0, Some("mg/dL"), Some(UCUM)));
        assert_eq!(o.reference_range[0].high.as_ref().unwrap().value, 99.0);
        assert_eq!(o.interpretation[0].coding[0].code.as_deref(), Some("H"));
        assert_eq!(o.status, "final");
        assert_eq!(o.effective_date_time.unwrap().to_string(), "2026-03-01T08:00:00Z");
    }

    #[test]
    fn maps_structured_numeric_and_coded_values() {
        let o = obx("OBX|1|SN|1234-5^Troponin^LN||<^0.01|ng/mL|<0.04||||F");
        let q = o.value_quantity.unwrap();
        assert_eq!((q.value, q.comparator.as_deref(), q.unit.as_deref()), (0.01, Some("<"), Some("ng/mL")));
        let o = obx("OBX|1|SN|1-1^Titer^LN||^1^:^128");
        assert_eq!(o.value_string.as_deref(), Some("1:128"));
        let o = obx("OBX|1|CWE|600-7^Culture^LN||112283007^E. coli^SCT||||||P");
        let concept = o.value_codeable_concept.unwrap();
        assert_eq!(concept.coding[0].system.as_deref(), Some(SNOMED));
        assert_eq!(o.status, "preliminary");
    }

    #[test]
    fn parses_reference_range_shapes() {
        let units = Message::parse("MSH|^~\\&|A\rOBX|1").unwrap();
        let f = units.segment("OBX").unwrap().field(6);
        assert_eq!(reference_range(">60", f).unwrap().low.unwrap().value, 60.0);
        let r = reference_range("-2.0-2.0", f).unwrap();
        assert_eq!((r.low.unwrap().value, r.high.unwrap().value), (-2.0, 2.0));
        assert_eq!(reference_range("negative", f).unwrap().text.as_deref(), Some("negative"));
    }

    #[test]
    fn bad_numeric_values_warn_instead_of_failing() {
        let text =
            "MSH|^~\\&|LAB|F|||20260301||ORU^R01|1|P|2.5.1\rPID|1||X1||||1990|Q\rOBX|1|NM|1^A^LN||high";
        let msg = Message::parse(text).unwrap();
        let conv = convert(&msg, &ConvertOptions::default()).unwrap();
        assert_eq!(conv.observations[0].value_string.as_deref(), Some("high"));
        assert_eq!(conv.warnings.len(), 2, "{:?}", conv.warnings);
        assert_eq!(conv.patient.birth_date.unwrap().to_string(), "1990");
    }

    #[test]
    fn rejects_unsupported_types_and_missing_pid() {
        let msg = Message::parse("MSH|^~\\&|A|B|||2026||SIU^S12|1|P|2.5").unwrap();
        assert!(matches!(
            convert(&msg, &ConvertOptions::default()),
            Err(ConvertError::UnsupportedMessageType(_))
        ));
        let msg = Message::parse("MSH|^~\\&|A|B|||2026||ADT^A01|1|P|2.5").unwrap();
        assert_eq!(
            convert(&msg, &ConvertOptions::default()).unwrap_err(),
            ConvertError::MissingSegment("PID")
        );
    }
}
