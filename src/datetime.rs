//! HL7 `DTM`/`TS` parsing, FHIR `date`/`dateTime` formatting and calendar arithmetic.

use std::fmt;

use serde::{Serialize, Serializer};

/// How much of a timestamp was specified in the source message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precision {
    /// `YYYY`
    Year,
    /// `YYYYMM`
    Month,
    /// `YYYYMMDD`
    Day,
    /// `YYYYMMDDHH[MM[SS]]`; FHIR always renders seconds once a time is present.
    Time,
}

/// A calendar date or date-time with optional UTC offset. Fractional seconds are dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    precision: Precision,
    /// Offset from UTC in minutes, when known.
    offset: Option<i16>,
}

impl DateTime {
    /// Parses an HL7 v2 `DTM`/`TS`/`DT` value: `YYYY[MM[DD[HH[MM[SS[.S+]]]]]][+/-ZZZZ]`.
    /// Only the first component is read, so `TS` values with a degree-of-precision component work.
    pub fn parse_hl7(value: &str) -> Option<Self> {
        let (body, offset) = match value.find(['+', '-']) {
            Some(i) => (&value[..i], Some(parse_hhmm_offset(&value[i..])?)),
            None => (value, None),
        };
        let digits = body.split('.').next()?;
        if !digits.bytes().all(|b| b.is_ascii_digit()) || ![4, 6, 8, 10, 12, 14].contains(&digits.len()) {
            return None;
        }
        let num = |range: std::ops::Range<usize>, default: u8| -> u8 {
            digits.get(range).map_or(default, |s| s.parse().unwrap_or(u8::MAX))
        };
        let precision = match digits.len() {
            4 => Precision::Year,
            6 => Precision::Month,
            8 => Precision::Day,
            _ => Precision::Time,
        };
        let dt = Self {
            year: digits[..4].parse().ok()?,
            month: num(4..6, 1),
            day: num(6..8, 1),
            hour: num(8..10, 0),
            minute: num(10..12, 0),
            second: num(12..14, 0),
            precision,
            offset,
        };
        let valid = (1..=12).contains(&dt.month)
            && dt.day >= 1
            && dt.day <= days_in_month(dt.year, dt.month)
            && dt.hour < 24
            && dt.minute < 60
            && dt.second < 60;
        valid.then_some(dt)
    }

    /// The precision of the source value.
    pub fn precision(&self) -> Precision {
        self.precision
    }

    /// The calendar year.
    pub fn year(&self) -> i32 {
        self.year
    }

    /// Truncates the value to at most day precision (used for FHIR `date` elements).
    pub fn to_date(mut self) -> Self {
        if self.precision == Precision::Time {
            self.precision = Precision::Day;
            self.offset = None;
        }
        self
    }

    /// Fills in a UTC offset (in minutes) for time-precision values that do not declare one.
    /// FHIR requires an offset whenever a time is present.
    pub fn with_default_offset(mut self, minutes: i16) -> Self {
        if self.precision == Precision::Time && self.offset.is_none() {
            self.offset = Some(minutes);
        }
        self
    }

    /// Moves the value by a whole number of days, keeping its precision and time of day.
    pub fn shift_days(self, days: i64) -> Self {
        let (year, month, day) = civil_from_days(days_from_civil(self.year, self.month, self.day) + days);
        Self { year, month, day, ..self }
    }

    /// Completed years between `self` (a birth date) and `at`.
    pub fn age_at(&self, at: &DateTime) -> i32 {
        let mut age = at.year - self.year;
        if (at.month, at.day) < (self.month, self.day) {
            age -= 1;
        }
        age
    }
}

impl fmt::Display for DateTime {
    /// Formats as a FHIR `date` or `dateTime`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}", self.year)?;
        if self.precision >= Precision::Month {
            write!(f, "-{:02}", self.month)?;
        }
        if self.precision >= Precision::Day {
            write!(f, "-{:02}", self.day)?;
        }
        if self.precision == Precision::Time {
            write!(f, "T{:02}:{:02}:{:02}", self.hour, self.minute, self.second)?;
            match self.offset.unwrap_or(0) {
                0 => write!(f, "Z")?,
                m => {
                    let sign = if m < 0 { '-' } else { '+' };
                    write!(f, "{sign}{:02}:{:02}", m.abs() / 60, m.abs() % 60)?;
                }
            }
        }
        Ok(())
    }
}

impl Serialize for DateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Parses a CLI-style UTC offset (`Z`, `+05:30`, `-0800`) into minutes.
pub fn parse_offset(value: &str) -> Option<i16> {
    if value.eq_ignore_ascii_case("z") {
        return Some(0);
    }
    parse_hhmm_offset(&value.replace(':', ""))
}

fn parse_hhmm_offset(value: &str) -> Option<i16> {
    let sign = match value.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = &value[1..];
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i16 = digits[..2].parse().ok()?;
    let minutes: i16 = digits[2..].parse().ok()?;
    (hours <= 14 && minutes < 60).then_some(sign * (hours * 60 + minutes))
}

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
pub fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date `(year, month, day)` for a count of days since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> DateTime {
        DateTime::parse_hl7(s).unwrap()
    }

    #[test]
    fn formats_each_precision_as_fhir() {
        assert_eq!(p("1980").to_string(), "1980");
        assert_eq!(p("198007").to_string(), "1980-07");
        assert_eq!(p("19800704").to_string(), "1980-07-04");
        assert_eq!(p("202601151030").to_string(), "2026-01-15T10:30:00Z");
        assert_eq!(p("20260115103045.123-0500").to_string(), "2026-01-15T10:30:45-05:00");
        assert_eq!(p("2026011510").with_default_offset(330).to_string(), "2026-01-15T10:00:00+05:30");
        assert_eq!(p("20260115103045+0100").to_date().to_string(), "2026-01-15");
    }

    #[test]
    fn rejects_invalid_values() {
        for bad in ["", "19", "2026131", "20260230", "20261301", "2026010125", "abcd", "20260101+99"] {
            assert!(DateTime::parse_hl7(bad).is_none(), "{bad}");
        }
        assert!(DateTime::parse_hl7("20240229").is_some());
    }

    #[test]
    fn shifts_across_month_year_and_leap_boundaries() {
        assert_eq!(p("20240228").shift_days(1).to_string(), "2024-02-29");
        assert_eq!(p("20241231").shift_days(1).to_string(), "2025-01-01");
        assert_eq!(p("20250301").shift_days(-365).to_string(), "2024-03-01");
        assert_eq!(p("202601151030").shift_days(-20).to_string(), "2025-12-26T10:30:00Z");
    }

    #[test]
    fn civil_round_trip() {
        for days in [-800_000, -1, 0, 1, 19_000, 2_000_000] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn computes_completed_years() {
        assert_eq!(p("19360615").age_at(&p("20260614")), 89);
        assert_eq!(p("19360615").age_at(&p("20260615")), 90);
    }

    #[test]
    fn parses_cli_offsets() {
        assert_eq!(parse_offset("Z"), Some(0));
        assert_eq!(parse_offset("+05:30"), Some(330));
        assert_eq!(parse_offset("-0800"), Some(-480));
        assert_eq!(parse_offset("05:00"), None);
    }
}
