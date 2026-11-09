//! Deterministic generator of synthetic HL7 v2 messages for benchmarks and demos.
//!
//! Every name, identifier and address is fabricated from small word lists; no output corresponds
//! to a real person. The same seed always produces the same corpus.

/// Families of `(LOINC code, display, unit, low, high)` used for generated lab results.
const LABS: [(&str, &str, &str, f64, f64); 8] = [
    ("2345-7", "Glucose [Mass/volume] in Serum or Plasma", "mg/dL", 70.0, 99.0),
    ("2951-2", "Sodium [Moles/volume] in Serum or Plasma", "mmol/L", 136.0, 145.0),
    ("2823-3", "Potassium [Moles/volume] in Serum or Plasma", "mmol/L", 3.5, 5.1),
    ("2160-0", "Creatinine [Mass/volume] in Serum or Plasma", "mg/dL", 0.6, 1.2),
    ("718-7", "Hemoglobin [Mass/volume] in Blood", "g/dL", 12.0, 17.5),
    ("6690-2", "Leukocytes [#/volume] in Blood by Automated count", "10*3/uL", 4.5, 11.0),
    ("777-3", "Platelets [#/volume] in Blood by Automated count", "10*3/uL", 150.0, 400.0),
    ("17861-6", "Calcium [Mass/volume] in Serum or Plasma", "mg/dL", 8.6, 10.3),
];
const FAMILY: [&str; 6] = ["TESTPATIENT", "SAMPLE", "FAKERSON", "EXAMPLE", "SYNTHETIC", "DEMO"];
const GIVEN: [&str; 6] = ["ALPHA", "BRAVO", "CHARLIE", "DELTA", "ECHO", "FOXTROT"];
const EVENTS: [(&str, &str); 4] = [("ADT", "A01"), ("ADT", "A04"), ("ADT", "A08"), ("ORU", "R01")];

/// A small xorshift64* generator, so the corpus does not depend on an external RNG crate.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Generates `count` synthetic messages (a mix of ADT^A01/A04/A08 and ORU^R01, CR-terminated).
pub fn messages(seed: u64, count: usize) -> Vec<String> {
    let mut rng = Rng(seed.max(1));
    (0..count).map(|i| message(&mut rng, i)).collect()
}

fn message(rng: &mut Rng, index: usize) -> String {
    let (code, event) = *rng.pick(&EVENTS);
    let patient = rng.below(500);
    let family = FAMILY[patient % FAMILY.len()];
    let given = GIVEN[(patient / FAMILY.len()) % GIVEN.len()];
    let birth_year = 1925 + (patient % 95);
    let day = 1 + rng.below(28);
    let hour = rng.below(24);
    let class = if code == "ORU" { "O" } else { *rng.pick(&["I", "O", "E"]) };
    let mut segments = vec![
        format!("MSH|^~\\&|SYNTH_EHR|TEST_FACILITY|FHIR_BRIDGE|TEST|202603{day:02}{hour:02}1500||{code}^{event}|SYN{index:06}|P|2.5.1"),
        format!("EVN|{event}|202603{day:02}{hour:02}1500"),
        format!(
            "PID|1||SYN{patient:05}^^^TESTHOSP^MR~000-00-{ssn:04}^^^USSSA^SS||{family}^{given}^Q||{birth_year}0{m}15|{sex}|||{street} Example St^Apt {apt}^Testville^ZZ^{zip:05}^USA^H||^PRN^PH^^1^555^{phone:07}|||S||ACC{index:06}|000-00-{ssn:04}",
            ssn = 1 + patient,
            m = 1 + patient % 9,
            sex = if patient % 2 == 1 { "M" } else { "F" },
            street = 100 + patient,
            apt = 1 + patient % 20,
            zip = 10_000 + patient * 37,
            phone = 5_550_100 + patient % 100,
        ),
        segment(
            "PV1",
            &[
                (1, "1".to_owned()),
                (2, class.to_owned()),
                (3, format!("WARD{}^{}^{}^TESTHOSP", rng.below(9), 100 + rng.below(300), rng.pick(&["A", "B"]))),
                (7, format!("9{:04}^CLINICIAN^{given}^^^DR", rng.below(50))),
                (10, "MED".to_owned()),
                (19, format!("V{index:07}")),
                (44, format!("202603{day:02}{hour:02}0000")),
            ],
        ),
    ];
    if code == "ORU" {
        segments.push(format!(
            "OBR|1|ORD{index:06}|FIL{index:06}|24323-8^Comprehensive metabolic panel^LN|||202603{day:02}{hour:02}0000"
        ));
        for set_id in 1..=(3 + rng.below(6)) {
            let (loinc, display, unit, low, high) = *rng.pick(&LABS);
            let spread = high - low;
            let value = low - spread * 0.25 + spread * 1.5 * (rng.below(1000) as f64 / 1000.0);
            let flag = if value > high {
                "H"
            } else if value < low {
                "L"
            } else {
                "N"
            };
            segments.push(format!(
                "OBX|{set_id}|NM|{loinc}^{display}^LN||{value:.1}|{unit}^{unit}^UCUM|{low}-{high}|{flag}|||F|||202603{day:02}{hour:02}3000"
            ));
        }
        segments.push("NTE|1||Synthetic result \\T\\ generated for benchmarking".to_owned());
    }
    segments.join("\r")
}

/// Builds a segment from `(field number, value)` pairs, leaving the other fields empty.
fn segment(id: &str, fields: &[(usize, String)]) -> String {
    let last = fields.iter().map(|(n, _)| *n).max().unwrap_or(0);
    let mut values = vec![""; last + 1];
    values[0] = id;
    for (n, value) in fields {
        values[*n] = value;
    }
    values.join("|")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hl7::Message;

    #[test]
    fn corpus_is_deterministic_and_parseable() {
        let a = messages(7, 50);
        assert_eq!(a, messages(7, 50));
        assert_ne!(a, messages(8, 50));
        for text in &a {
            let msg = Message::parse(text).unwrap();
            assert!(crate::validate::validate(&msg).is_empty(), "{text}");
        }
    }
}
