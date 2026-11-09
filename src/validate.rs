//! Structural and semantic checks on a parsed HL7 v2 message.

use std::fmt;

use crate::datetime::DateTime;
use crate::fhir::convert::is_supported;
use crate::hl7::{Message, Segment};

/// How serious a validation finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The message is invalid or cannot be converted.
    Error,
    /// The message can be converted but some data will be lost or guessed.
    Warning,
}

/// One validation finding, located by segment, line and field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    pub segment: String,
    pub line: usize,
    pub field: usize,
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{level}: {}-{} (line {}): {}", self.segment, self.field, self.line, self.message)
    }
}

/// Validates a message and returns all findings (empty when the message is clean).
pub fn validate(msg: &Message<'_>) -> Vec<Issue> {
    let mut v = Validator::default();
    let msh = msg.msh();
    for (field, name) in [(9, "message type"), (10, "message control ID"), (12, "version ID")] {
        v.require(msh, field, name);
    }
    let version = msh.field(12).component(1).raw();
    if !version.is_empty() && !version.starts_with("2.") {
        v.push(Severity::Warning, msh, 12, format!("unexpected HL7 version {version:?}"));
    }
    v.timestamp(msh, 7);

    let (code, event) = msg.message_type();
    if !code.is_empty() && !is_supported(code, event) {
        v.push(
            Severity::Warning,
            msh,
            9,
            format!("{code}^{event} is valid HL7 but cannot be converted to FHIR"),
        );
    }
    let needs_pid = matches!(code, "ADT" | "ORU");
    match msg.segment("PID") {
        Some(pid) => {
            v.require(pid, 3, "patient identifier");
            v.timestamp(pid, 7);
            if !["", "M", "F", "O", "U", "A", "N"].contains(&pid.field(8).raw()) {
                v.push(Severity::Warning, pid, 8, "unknown administrative sex".to_owned());
            }
        }
        None if needs_pid => v.push(Severity::Error, msh, 9, "PID segment is required".to_owned()),
        None => {}
    }
    if code == "ADT" && msg.segment("PV1").is_none() {
        v.push(Severity::Error, msh, 9, "ADT messages require a PV1 segment".to_owned());
    }
    for segment in msg.segments() {
        match segment.id() {
            "PV1" => {
                v.timestamp(segment, 44);
                v.timestamp(segment, 45);
            }
            "OBR" => v.timestamp(segment, 7),
            "OBX" => v.obx(segment),
            _ => {}
        }
    }
    v.issues
}

#[derive(Default)]
struct Validator {
    issues: Vec<Issue>,
}

impl Validator {
    fn push(&mut self, severity: Severity, segment: &Segment<'_>, field: usize, message: String) {
        self.issues.push(Issue {
            severity,
            segment: segment.id().to_owned(),
            line: segment.line(),
            field,
            message,
        });
    }

    fn require(&mut self, segment: &Segment<'_>, field: usize, name: &str) {
        if segment.field(field).is_empty() {
            self.push(Severity::Error, segment, field, format!("{name} is required"));
        }
    }

    fn timestamp(&mut self, segment: &Segment<'_>, field: usize) {
        let raw = segment.field(field).component(1).raw();
        if !raw.is_empty() && DateTime::parse_hl7(raw).is_none() {
            self.push(Severity::Error, segment, field, format!("invalid timestamp {raw:?}"));
        }
    }

    fn obx(&mut self, obx: &Segment<'_>) {
        let value_type = obx.field(2).raw();
        let value = obx.field(5);
        if value_type.is_empty() && !value.is_empty() {
            self.push(Severity::Error, obx, 2, "value type is required when OBX-5 is valued".to_owned());
        }
        self.require(obx, 3, "observation identifier");
        if value_type == "NM" && !value.is_empty() && value.value().trim().parse::<f64>().is_err() {
            self.push(Severity::Error, obx, 5, format!("{:?} is not a valid NM value", value.raw()));
        }
        if !["", "F", "C", "P", "R", "S", "I", "X", "D", "W"].contains(&obx.field(11).raw()) {
            self.push(Severity::Warning, obx, 11, "unknown result status".to_owned());
        }
        self.timestamp(obx, 14);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_message_has_no_issues() {
        let text = "MSH|^~\\&|A|B|||20260101||ADT^A04|C1|P|2.5.1\rPID|1||M1||X^Y||19700101|F\rPV1|1|O";
        assert!(validate(&Message::parse(text).unwrap()).is_empty());
    }

    #[test]
    fn reports_missing_and_invalid_fields() {
        let text = "MSH|^~\\&|A|B|||2026-01-01||ORU^R01||P|3.0\rPID|1||||||1970\rOBX|1|NM|1^X^LN||abc||||||Z";
        let issues = validate(&Message::parse(text).unwrap());
        let summary: Vec<_> = issues.iter().map(|i| (i.severity, i.segment.as_str(), i.field)).collect();
        assert_eq!(
            summary,
            [
                (Severity::Error, "MSH", 10),
                (Severity::Warning, "MSH", 12),
                (Severity::Error, "MSH", 7),
                (Severity::Error, "PID", 3),
                (Severity::Error, "OBX", 5),
                (Severity::Warning, "OBX", 11),
            ]
        );
        assert_eq!(issues[3].to_string(), "error: PID-3 (line 2): patient identifier is required");
    }
}
