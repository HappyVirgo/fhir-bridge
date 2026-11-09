//! End-to-end tests over the synthetic HL7 fixtures in `tests/fixtures`.

use std::collections::HashSet;
use std::fs;

use fhir_bridge::deid::{Deidentifier, AGE_CATEGORY_URL, PSEUDONYM_SYSTEM};
use fhir_bridge::fhir::{self, ConvertError, ConvertOptions};
use fhir_bridge::hl7::{split_messages, Message, ParseErrorKind};
use fhir_bridge::validate::{validate, Severity};
use serde_json::Value;

fn fixture(name: &str) -> String {
    fs::read_to_string(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn bundle_json(text: &str, deid: Option<&Deidentifier>) -> Value {
    let message = Message::parse(text).unwrap();
    let (bundle, _) = fhir::message_to_bundle(&message, &ConvertOptions::default(), deid).unwrap();
    serde_json::to_value(bundle).unwrap()
}

fn convert(name: &str) -> Value {
    bundle_json(&fixture(name), None)
}

fn deidentify(name: &str, salt: &str) -> Value {
    bundle_json(&fixture(name), Some(&Deidentifier::new(salt).unwrap()))
}

fn resources<'a>(bundle: &'a Value, kind: &str) -> Vec<&'a Value> {
    bundle["entry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["resource"])
        .filter(|r| r["resourceType"] == kind)
        .collect()
}

fn patient(bundle: &Value) -> &Value {
    resources(bundle, "Patient")[0]
}

#[test]
fn adt_a01_maps_patient_and_encounter() {
    let bundle = convert("adt_a01_inpatient.hl7");
    assert_eq!(bundle["type"], "transaction");
    let p = patient(&bundle);
    assert_eq!(p["name"][0]["family"], "SAMPLE");
    assert_eq!(p["name"][0]["given"], serde_json::json!(["ALICE", "MARIE"]));
    assert_eq!(p["birthDate"], "1980-04-15");
    assert_eq!(p["gender"], "female");
    assert_eq!(p["identifier"][0]["value"], "TST100001");
    assert_eq!(p["identifier"][1]["system"], "http://hl7.org/fhir/sid/us-ssn");
    assert_eq!(p["telecom"][1]["system"], "email");
    assert_eq!(p["address"][0]["postalCode"], "02139");
    let e = resources(&bundle, "Encounter")[0];
    assert_eq!(e["class"]["code"], "IMP");
    assert_eq!(e["status"], "in-progress");
    assert_eq!(e["period"]["start"], "2026-03-12T08:00:00-05:00");
    assert_eq!(e["participant"][0]["individual"]["display"], "DR ROBERT CLINICIAN");
}

#[test]
fn encounter_status_and_class_follow_pv1() {
    let a08 = convert("adt_a08_update_discharged.hl7");
    let e = resources(&a08, "Encounter")[0];
    assert_eq!(e["status"], "finished");
    assert_eq!(e["period"]["end"], "2026-03-15T16:30:00-05:00");
    let a04 = convert("adt_a04_outpatient.hl7");
    assert_eq!(resources(&a04, "Encounter")[0]["class"]["code"], "AMB");
    assert_eq!(patient(&a04)["name"][0]["suffix"][0], "JR");
    assert_eq!(patient(&a04)["telecom"][0]["value"], "(555)010-0222");
}

#[test]
fn repetitions_produce_multiple_identifiers_names_and_addresses() {
    let bundle = convert("adt_a01_repetitions.hl7");
    let p = patient(&bundle);
    assert_eq!(p["identifier"][1]["system"], "urn:oid:2.16.840.1.113883.999.1");
    assert_eq!(p["name"].as_array().unwrap().len(), 2);
    assert_eq!(p["name"][1]["use"], "usual");
    assert_eq!(p["address"][1]["use"], "work");
    assert_eq!(p["maritalStatus"]["coding"][0]["code"], "W");
    let e = resources(&bundle, "Encounter")[0];
    assert_eq!(e["class"]["code"], "EMER");
    assert_eq!(e["participant"].as_array().unwrap().len(), 2);
}

#[test]
fn escape_sequences_are_decoded() {
    let bundle = convert("adt_a01_escapes.hl7");
    let p = patient(&bundle);
    assert_eq!(p["name"][0]["family"], "O&TESTER");
    assert_eq!(p["name"][0]["given"][0], "DAN^NY");
    assert_eq!(p["address"][0]["line"][0], "5 Fake Rd & Main|Rear");
    let obs = resources(&bundle, "Observation");
    assert_eq!(obs.len(), 3);
    assert_eq!(obs[2]["valueString"], "Measured with cuff | left arm\\ seated");
    // ADT observations are not lab results, and timestamps without an offset default to UTC.
    assert!(obs[0].get("category").is_none());
    assert_eq!(resources(&bundle, "Encounter")[0]["period"]["start"], "2026-06-10T11:30:00Z");
}

#[test]
fn line_ending_variants_convert_identically() {
    for name in ["adt_a04_lf.hl7", "adt_a08_crlf.hl7"] {
        let text = fixture(name);
        let normalised = text.replace("\r\n", "\r").replace('\n', "\r");
        assert_ne!(text, normalised, "{name} should use non-CR line endings");
        assert_eq!(bundle_json(&text, None), bundle_json(&normalised, None));
    }
}

#[test]
fn custom_delimiters_are_honoured() {
    let bundle = convert("adt_a01_custom_delimiters.hl7");
    let p = patient(&bundle);
    assert_eq!(p["identifier"].as_array().unwrap().len(), 2);
    assert_eq!(p["name"][0]["given"][0], "DEE$DEE");
    assert_eq!(p["address"][0]["city"], "Altville");
}

#[test]
fn oru_r01_maps_lab_observations() {
    let bundle = convert("oru_r01_cbc.hl7");
    let obs = resources(&bundle, "Observation");
    assert_eq!(obs.len(), 4);
    let wbc = obs[0];
    assert_eq!(wbc["category"][0]["coding"][0]["code"], "laboratory");
    assert_eq!(wbc["code"]["coding"][0]["system"], "http://loinc.org");
    assert_eq!(wbc["code"]["coding"][0]["code"], "6690-2");
    assert_eq!(wbc["valueQuantity"]["value"], 12.4);
    assert_eq!(wbc["valueQuantity"]["system"], "http://unitsofmeasure.org");
    assert_eq!(wbc["valueQuantity"]["code"], "10*3/uL");
    assert_eq!(wbc["referenceRange"][0]["high"]["value"], 11.0);
    assert_eq!(wbc["interpretation"][0]["coding"][0]["code"], "H");
    assert_eq!(wbc["effectiveDateTime"], "2026-03-13T05:55:00-05:00");
    // OBR-7 is the fallback effective time when OBX-14 is empty.
    let bmp = convert("oru_r01_bmp.hl7");
    let glucose = resources(&bmp, "Observation")[0];
    assert_eq!(glucose["effectiveDateTime"], "2026-03-14T05:30:00-05:00");
    assert!(glucose.get("encounter").is_none(), "no PV1, so no encounter reference");
    assert_eq!(resources(&bmp, "Observation")[3]["status"], "corrected");
}

#[test]
fn oru_r01_maps_each_value_type() {
    let bundle = convert("oru_r01_mixed_types.hl7");
    let obs = resources(&bundle, "Observation");
    assert_eq!(obs[0]["valueQuantity"]["comparator"], "<");
    assert_eq!(obs[0]["referenceRange"][0]["high"]["value"], 0.04);
    assert_eq!(obs[1]["valueCodeableConcept"]["coding"][0]["system"], "http://snomed.info/sct");
    assert_eq!(obs[1]["status"], "preliminary");
    assert_eq!(obs[2]["valueString"], "Line one of the report\nLine two of the report");
    assert_eq!(obs[3]["valueString"], "Yellow");
    assert_eq!(obs[4]["valueString"], "1:128");
    assert_eq!(obs[5]["status"], "cancelled");
    assert!(obs[5]["valueQuantity"].get("system").is_none(), "non-UCUM units carry no system");
    assert_eq!(patient(&bundle)["gender"], "unknown");
}

fn collect_references(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("reference") {
                out.push(r.clone());
            }
            map.values().for_each(|v| collect_references(v, out));
        }
        Value::Array(items) => items.iter().for_each(|v| collect_references(v, out)),
        _ => {}
    }
}

#[test]
fn every_reference_resolves_within_the_bundle() {
    for name in ["adt_a01_inpatient.hl7", "oru_r01_cbc.hl7", "adt_a01_escapes.hl7"] {
        let bundle = convert(name);
        let urls: HashSet<_> = bundle["entry"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["fullUrl"].as_str().unwrap().to_owned())
            .collect();
        let mut refs = Vec::new();
        collect_references(&bundle, &mut refs);
        assert!(!refs.is_empty());
        assert!(refs.iter().all(|r| urls.contains(r)), "{name}: {refs:?}");
    }
}

#[test]
fn conversion_is_deterministic() {
    assert_eq!(convert("oru_r01_cbc.hl7"), convert("oru_r01_cbc.hl7"));
    assert_eq!(deidentify("oru_r01_cbc.hl7", "s"), deidentify("oru_r01_cbc.hl7", "s"));
}

#[test]
fn batch_files_split_into_messages() {
    let text = fixture("batch_two_messages.hl7");
    let messages = split_messages(&text);
    assert_eq!(messages.len(), 2);
    let names: Vec<_> =
        messages.iter().map(|m| patient(&bundle_json(m, None))["name"][0]["family"].clone()).collect();
    assert_eq!(names, ["BATCHONE", "BATCHTWO"]);
}

#[test]
fn malformed_messages_report_positions() {
    let cases = [
        ("malformed_missing_msh.hl7", 1, "MissingMsh"),
        ("malformed_bad_segment_id.hl7", 3, "InvalidSegmentId"),
        ("malformed_bad_encoding.hl7", 1, "InvalidEncodingCharacters"),
        ("malformed_truncated_msh.hl7", 1, "TruncatedMsh"),
    ];
    for (name, line, kind) in cases {
        let err = Message::parse(&fixture(name)).unwrap_err();
        assert_eq!(err.line, line, "{name}");
        assert!(format!("{:?}", err.kind).starts_with(kind), "{name}: {err}");
    }
    let err = Message::parse(&fixture("malformed_bad_segment_id.hl7")).unwrap_err();
    assert_eq!(err.kind, ParseErrorKind::InvalidSegmentId("Pid".into()));
    assert_eq!(err.to_string(), "line 3, segment 3: invalid segment identifier \"Pid\"");
}

#[test]
fn semantic_problems_are_reported_by_validate() {
    let text = fixture("invalid_semantics.hl7");
    let issues = validate(&Message::parse(&text).unwrap());
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| format!("{}-{}", i.segment, i.field))
        .collect();
    assert_eq!(errors, ["MSH-10", "PID-3", "PID-7", "OBX-5"]);
}

#[test]
fn unsupported_message_types_are_rejected() {
    let text = fixture("unsupported_siu_s12.hl7");
    let err = fhir::convert(&Message::parse(&text).unwrap(), &ConvertOptions::default()).unwrap_err();
    assert_eq!(err, ConvertError::UnsupportedMessageType("SIU^S12".into()));
}

#[test]
fn deidentified_output_contains_no_direct_identifiers() {
    let phi = [
        "SAMPLE",
        "ALICE",
        "MARIE",
        "Example Street",
        "Springfield",
        "02139",
        "0100123",
        "0100456",
        "alice.sample",
        "000-00-0001",
        "TST100001",
        "V0000001",
        "CLINICIAN",
        "1980-04-15",
        "ACC0001",
    ];
    for name in ["adt_a01_inpatient.hl7", "adt_a08_update_discharged.hl7", "oru_r01_cbc.hl7"] {
        let json = deidentify(name, "test-salt").to_string();
        for value in phi {
            assert!(!json.contains(value), "{name} leaks {value}: {json}");
        }
        assert!(json.contains(PSEUDONYM_SYSTEM));
    }
    let p = deidentify("adt_a01_inpatient.hl7", "test-salt");
    assert_eq!(patient(&p)["address"][0]["postalCode"], "021");
    assert_eq!(patient(&p)["identifier"].as_array().unwrap().len(), 1, "SSN removed");
}

#[test]
fn pseudonyms_and_date_shifts_are_stable_per_patient() {
    let adt = deidentify("adt_a01_inpatient.hl7", "test-salt");
    let lab = deidentify("oru_r01_bmp.hl7", "test-salt");
    let other_patient = deidentify("adt_a04_outpatient.hl7", "test-salt");
    let id = |b: &Value| patient(b)["identifier"][0]["value"].clone();
    assert_eq!(id(&adt), id(&lab));
    assert_ne!(id(&adt), id(&other_patient));
    assert_eq!(patient(&adt)["birthDate"], patient(&lab)["birthDate"]);
    assert_ne!(patient(&adt)["birthDate"], "1980-04-15");
    assert_ne!(id(&adt), id(&deidentify("adt_a01_inpatient.hl7", "other-salt")));
}

#[test]
fn date_shift_preserves_intervals() {
    let original = convert("oru_r01_cbc.hl7");
    let shifted = deidentify("oru_r01_cbc.hl7", "interval-salt");
    let day = |b: &Value, kind: &str, path: &[&str]| -> String {
        let mut v = resources(b, kind)[0];
        for key in path {
            v = &v[*key];
        }
        v.as_str().unwrap()[..10].to_owned()
    };
    let days = |date: &str| {
        let parts: Vec<i32> = date.split('-').map(|p| p.parse().unwrap()).collect();
        fhir_bridge::datetime::days_from_civil(parts[0], parts[1] as u8, parts[2] as u8)
    };
    let interval = |b: &Value| {
        days(&day(b, "Observation", &["effectiveDateTime"]))
            - days(&day(b, "Encounter", &["period", "start"]))
    };
    assert_eq!(interval(&original), interval(&shifted));
    assert_ne!(
        day(&original, "Encounter", &["period", "start"]),
        day(&shifted, "Encounter", &["period", "start"])
    );
}

#[test]
fn ages_over_89_are_generalised() {
    let bundle = deidentify("adt_a01_elderly.hl7", "test-salt");
    let p = patient(&bundle);
    assert!(p.get("birthDate").is_none());
    assert_eq!(p["extension"][0]["url"], AGE_CATEGORY_URL);
    assert_eq!(p["extension"][0]["valueString"], "90+");
    assert_eq!(p["address"][0]["postalCode"], "000", "821 is a restricted ZIP3 prefix");
}

#[test]
fn free_text_results_are_masked_when_deidentified() {
    let bundle = deidentify("oru_r01_mixed_types.hl7", "test-salt");
    let obs = resources(&bundle, "Observation");
    assert!(obs[2].get("valueString").is_none());
    assert_eq!(obs[2]["dataAbsentReason"]["coding"][0]["code"], "masked");
    assert_eq!(obs[0]["valueQuantity"]["value"], 0.01, "numeric results are kept");
}
