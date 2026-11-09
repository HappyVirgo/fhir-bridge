//! Tests that run the compiled `fhir-bridge` binary.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fhir-bridge"))
        .args(args)
        .env_remove("FHIR_BRIDGE_SALT")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn convert_prints_a_bundle() {
    let input = fixture("oru_r01_cbc.hl7");
    let out = run(&["convert", input.to_str().unwrap()]);
    assert!(out.status.success(), "{}", stderr(&out));
    let bundle: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(bundle["resourceType"], "Bundle");
    assert_eq!(bundle["entry"].as_array().unwrap().len(), 6);
}

#[test]
fn convert_honours_tz_and_reads_salt_from_env() {
    let input = fixture("adt_a01_escapes.hl7");
    let out = run(&["convert", input.to_str().unwrap(), "--tz", "-08:00", "--compact"]);
    assert!(stdout(&out).contains("T11:30:00-08:00"), "{}", stdout(&out));

    let out = Command::new(env!("CARGO_BIN_EXE_fhir-bridge"))
        .args(["convert", input.to_str().unwrap(), "--deidentify"])
        .env("FHIR_BRIDGE_SALT", "env-salt")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(!stdout(&out).contains("TESTER"));
}

#[test]
fn deidentify_without_salt_fails() {
    let input = fixture("adt_a01_inpatient.hl7");
    let out = run(&["convert", input.to_str().unwrap(), "--deidentify"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("salt"), "{}", stderr(&out));
}

#[test]
fn convert_reports_parse_errors_with_position() {
    let input = fixture("malformed_bad_segment_id.hl7");
    let out = run(&["convert", input.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("line 3, segment 3"), "{}", stderr(&out));
}

#[test]
fn multi_message_files_produce_an_array() {
    let input = fixture("batch_two_messages.hl7");
    let out = run(&["convert", input.to_str().unwrap()]);
    let bundles: Vec<serde_json::Value> = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(bundles.len(), 2);
}

#[test]
fn validate_flags_only_the_bad_fixtures() {
    let dir = fixture("");
    let out = run(&["validate", dir.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    let invalid: Vec<_> = text.lines().filter(|l| l.starts_with("INVALID")).collect();
    assert_eq!(invalid.len(), 5, "{text}");
    assert!(invalid.iter().all(|l| l.contains("malformed_") || l.contains("invalid_semantics")));

    let good = fixture("adt_a01_inpatient.hl7");
    assert!(run(&["validate", good.to_str().unwrap()]).status.success());
}

#[test]
fn batch_converts_a_directory() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("batch_test");
    let (input, output) = (root.join("in"), root.join("out"));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&input).unwrap();
    for name in ["adt_a01_inpatient.hl7", "oru_r01_bmp.hl7", "batch_two_messages.hl7"] {
        fs::copy(fixture(name), input.join(name)).unwrap();
    }
    let out = run(&[
        "batch",
        input.to_str().unwrap(),
        "--out-dir",
        output.to_str().unwrap(),
        "--deidentify",
        "--salt",
        "batch-salt",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("converted 4 message(s); 0 file(s) failed"), "{}", stdout(&out));
    let json = fs::read_to_string(output.join("oru_r01_bmp.json")).unwrap();
    assert!(json.contains("urn:fhir-bridge:pseudonym") && !json.contains("SAMPLE"));
}

#[test]
fn generate_writes_a_valid_corpus() {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("corpus.hl7");
    let out = run(&["generate", "--count", "25", "-o", path.to_str().unwrap()]);
    assert!(out.status.success());
    assert!(run(&["validate", path.to_str().unwrap()]).status.success());
}
