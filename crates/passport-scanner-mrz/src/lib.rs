//! ICAO 9303 machine-readable zone parsing.
//!
//! Covers the three document formats that matter for identity documents:
//!
//! | Format | Layout | Typical document |
//! |--------|--------|------------------|
//! | [`MrzFormat::Td1`] | 3 × 30 | ID card, e.g. the German Personalausweis |
//! | [`MrzFormat::Td2`] | 2 × 36 | older travel documents |
//! | [`MrzFormat::Td3`] | 2 × 44 | passport |
//!
//! The parser is pure: it takes the already-recognised character grid and produces
//! fields with their check-digit verdicts. Turning pixels into characters is the job of
//! a [`passport_scanner_core::DocumentRecognizer`] implementation that calls into this crate.

#![forbid(unsafe_code)]

pub mod check;

use passport_scanner_core::{Document, Error, Field, FieldKind, Result};

/// Which MRZ layout a document uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MrzFormat {
    /// Three lines of thirty characters.
    Td1,
    /// Two lines of thirty-six characters.
    Td2,
    /// Two lines of forty-four characters.
    Td3,
}

impl MrzFormat {
    /// Deduce the format from the shape of the input.
    #[must_use]
    pub fn detect(lines: &[&str]) -> Option<Self> {
        match (lines.len(), lines.first().map(|l| l.len())) {
            (3, Some(30)) => Some(Self::Td1),
            (2, Some(36)) => Some(Self::Td2),
            (2, Some(44)) => Some(Self::Td3),
            _ => None,
        }
    }

    /// Line length this format requires.
    #[must_use]
    pub const fn line_length(self) -> usize {
        match self {
            Self::Td1 => 30,
            Self::Td2 => 36,
            Self::Td3 => 44,
        }
    }
}

/// Parse a machine-readable zone.
///
/// `lines` must already be trimmed to the zone itself, in reading order.
pub fn parse(lines: &[&str]) -> Result<Document> {
    let format = MrzFormat::detect(lines)
        .ok_or_else(|| Error::Malformed("unrecognised MRZ shape".to_owned()))?;

    for (i, line) in lines.iter().enumerate() {
        if line.len() != format.line_length() {
            return Err(Error::Malformed(format!(
                "line {} is {} characters, expected {}",
                i + 1,
                line.len(),
                format.line_length()
            )));
        }
        if !line.bytes().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'<') {
            return Err(Error::Malformed(
                "MRZ contains characters outside its ASCII alphabet".into(),
            ));
        }
    }
    validate_structure(format, lines)?;

    let mut doc = match format {
        MrzFormat::Td1 => parse_td1(lines)?,
        MrzFormat::Td2 => parse_td2(lines)?,
        MrzFormat::Td3 => parse_td3(lines)?,
    };

    // The composite covers optional data as well as the individually checked
    // fields. Names have no check digit and must never be inferred from it.
    let (composite, digit) = match format {
        MrzFormat::Td1 => (
            format!(
                "{}{}{}{}",
                &lines[0][5..30],
                &lines[1][0..7],
                &lines[1][8..15],
                &lines[1][18..29]
            ),
            lines[1].as_bytes()[29],
        ),
        MrzFormat::Td2 => (
            format!("{}{}{}", &lines[1][0..10], &lines[1][13..20], &lines[1][21..35]),
            lines[1].as_bytes()[35],
        ),
        MrzFormat::Td3 => (
            format!("{}{}{}", &lines[1][0..10], &lines[1][13..20], &lines[1][21..43]),
            lines[1].as_bytes()[43],
        ),
    };
    doc.insert(Field::checked(
        FieldKind::MrzLine,
        lines.join("\n"),
        check::verify(&composite, digit as char),
    ));
    Ok(doc)
}

/// Validate row roles before slicing fields (ICAO 9303 parts 4, 5 and 6).
/// Length alone does not distinguish a name row from a document-number row.
fn validate_structure(format: MrzFormat, lines: &[&str]) -> Result<()> {
    let alpha = |s: &str| s.bytes().all(|b| b.is_ascii_uppercase() || b == b'<');
    let code = |s: &str| alpha(s) && s.bytes().any(|b| b.is_ascii_uppercase());
    let name = |s: &str| {
        if !alpha(s) || !s.as_bytes()[0].is_ascii_uppercase() {
            return false;
        }
        // After the primary/secondary separator, a second run of two fillers
        // marks padding, not another name. Do not turn OCR noise in padding
        // into an extra personal-name component.
        s.split_once("<<").is_none_or(|(_, given)| {
            given.split_once("<<").is_none_or(|(_, tail)| tail.bytes().all(|b| b == b'<'))
        })
    };
    let date = |s: &str| {
        s.bytes().all(|b| b.is_ascii_digit() || b == b'<')
            && (s.contains('<') || valid_calendar_date(s))
    };
    let digit = |b: u8| b.is_ascii_digit();
    let sex = |b: u8| matches!(b, b'M' | b'F' | b'<');
    let l1 = lines[0];
    let l2 = lines[1];
    let header = code(&l1[2..5]) && alpha(&l1[1..2]);
    let valid = match format {
        MrzFormat::Td1 => {
            header
                && matches!(l1.as_bytes()[0], b'A' | b'C' | b'I')
                && l1[5..14].bytes().any(|b| b != b'<')
                && digit(l1.as_bytes()[14])
                && date(&l2[0..6])
                && digit(l2.as_bytes()[6])
                && sex(l2.as_bytes()[7])
                && date(&l2[8..14])
                && digit(l2.as_bytes()[14])
                && code(&l2[15..18])
                && digit(l2.as_bytes()[29])
                && name(lines[2])
        }
        MrzFormat::Td2 | MrzFormat::Td3 => {
            header
                && if format == MrzFormat::Td3 {
                    l1.starts_with('P')
                } else {
                    matches!(l1.as_bytes()[0], b'A' | b'C' | b'I')
                }
                && name(&l1[5..])
                && l2[0..9].bytes().any(|b| b != b'<')
                && digit(l2.as_bytes()[9])
                && code(&l2[10..13])
                && date(&l2[13..19])
                && digit(l2.as_bytes()[19])
                && sex(l2.as_bytes()[20])
                && date(&l2[21..27])
                && digit(l2.as_bytes()[27])
                && digit(*l2.as_bytes().last().unwrap())
        }
    };
    if valid {
        Ok(())
    } else {
        // Extended document numbers with a relocated check digit are currently
        // unsupported, rather than silently returning a truncated number.
        Err(Error::Malformed("invalid or unsupported MRZ row structure".into()))
    }
}

fn valid_calendar_date(value: &str) -> bool {
    let Ok(month) = value[2..4].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[4..6].parse::<u32>() else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if value[0..2].parse::<u32>().is_ok_and(|y| y % 4 == 0) {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    (1..=days).contains(&day)
}

/// Strip filler characters and turn the name separator into a readable form.
fn clean(field: &str) -> String {
    field.trim_end_matches('<').replace('<', " ").trim().to_owned()
}

/// Split the `SURNAME<<GIVEN<NAMES` composite.
fn split_names(field: &str) -> (String, String) {
    match field.split_once("<<") {
        Some((surname, given)) => (clean(surname), clean(given)),
        None => (clean(field), String::new()),
    }
}

/// Expand a two-digit MRZ year into ISO 8601 using the standard's sliding window.
fn iso_date(yymmdd: &str, pivot: u32) -> Option<String> {
    if yymmdd.len() != 6 || !yymmdd.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let yy: u32 = yymmdd[0..2].parse().ok()?;
    let century = if yy > pivot { 1900 } else { 2000 };
    Some(format!("{}-{}-{}", century + yy, &yymmdd[2..4], &yymmdd[4..6]))
}

fn push_dates(doc: &mut Document, birth: &str, birth_cd: char, expiry: &str, expiry_cd: char) {
    // Births are always in the past; expiries run into the future.
    let born = iso_date(birth, 30).unwrap_or_else(|| birth.to_owned());
    let expires = iso_date(expiry, 99).unwrap_or_else(|| expiry.to_owned());
    doc.insert(Field::checked(FieldKind::DateOfBirth, born, check::verify(birth, birth_cd)));
    doc.insert(Field::checked(FieldKind::DateOfExpiry, expires, check::verify(expiry, expiry_cd)));
}

fn parse_td3(lines: &[&str]) -> Result<Document> {
    let (l1, l2) = (lines[0], lines[1]);
    let mut doc = Document::default();

    doc.insert(Field::plain(FieldKind::DocumentType, clean(&l1[0..2])));
    doc.insert(Field::plain(FieldKind::IssuingState, clean(&l1[2..5])));
    let (surname, given) = split_names(&l1[5..44]);
    doc.insert(Field::plain(FieldKind::Surname, surname));
    doc.insert(Field::plain(FieldKind::GivenName, given));

    let number = &l2[0..9];
    doc.insert(Field::checked(
        FieldKind::DocumentNumber,
        clean(number),
        check::verify(number, l2.as_bytes()[9] as char),
    ));
    doc.insert(Field::plain(FieldKind::Nationality, clean(&l2[10..13])));
    push_dates(
        &mut doc,
        &l2[13..19],
        l2.as_bytes()[19] as char,
        &l2[21..27],
        l2.as_bytes()[27] as char,
    );
    doc.insert(Field::plain(FieldKind::Sex, clean(&l2[20..21])));
    doc.insert(Field::checked(
        FieldKind::OptionalData,
        clean(&l2[28..42]),
        check::verify(&l2[28..42], l2.as_bytes()[42] as char),
    ));

    Ok(doc)
}

fn parse_td2(lines: &[&str]) -> Result<Document> {
    let (l1, l2) = (lines[0], lines[1]);
    let mut doc = Document::default();

    doc.insert(Field::plain(FieldKind::DocumentType, clean(&l1[0..2])));
    doc.insert(Field::plain(FieldKind::IssuingState, clean(&l1[2..5])));
    let (surname, given) = split_names(&l1[5..36]);
    doc.insert(Field::plain(FieldKind::Surname, surname));
    doc.insert(Field::plain(FieldKind::GivenName, given));

    let number = &l2[0..9];
    doc.insert(Field::checked(
        FieldKind::DocumentNumber,
        clean(number),
        check::verify(number, l2.as_bytes()[9] as char),
    ));
    doc.insert(Field::plain(FieldKind::Nationality, clean(&l2[10..13])));
    push_dates(
        &mut doc,
        &l2[13..19],
        l2.as_bytes()[19] as char,
        &l2[21..27],
        l2.as_bytes()[27] as char,
    );
    doc.insert(Field::plain(FieldKind::Sex, clean(&l2[20..21])));
    doc.insert(Field::plain(FieldKind::OptionalData, clean(&l2[28..35])));

    Ok(doc)
}

fn parse_td1(lines: &[&str]) -> Result<Document> {
    let (l1, l2, l3) = (lines[0], lines[1], lines[2]);
    let mut doc = Document::default();

    doc.insert(Field::plain(FieldKind::DocumentType, clean(&l1[0..2])));
    doc.insert(Field::plain(FieldKind::IssuingState, clean(&l1[2..5])));

    let number = &l1[5..14];
    doc.insert(Field::checked(
        FieldKind::DocumentNumber,
        clean(number),
        check::verify(number, l1.as_bytes()[14] as char),
    ));

    push_dates(
        &mut doc,
        &l2[0..6],
        l2.as_bytes()[6] as char,
        &l2[8..14],
        l2.as_bytes()[14] as char,
    );
    doc.insert(Field::plain(FieldKind::Sex, clean(&l2[7..8])));
    doc.insert(Field::plain(FieldKind::Nationality, clean(&l2[15..18])));

    let (surname, given) = split_names(l3);
    doc.insert(Field::plain(FieldKind::Surname, surname));
    doc.insert(Field::plain(FieldKind::GivenName, given));

    let optional = format!("{}{}", clean(&l1[15..30]), clean(&l2[18..29]));
    if !optional.is_empty() {
        doc.insert(Field::plain(FieldKind::OptionalData, optional));
    }

    Ok(doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    // All test vectors below are the fictitious specimen records printed in ICAO 9303.
    // They contain no real personal data.

    #[test]
    fn parses_the_specimen_passport() {
        let lines = [
            "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<",
            "L898902C36UTO7408122F1204159ZE184226B<<<<<10",
        ];
        let doc = parse(&lines).expect("specimen parses");

        assert_eq!(doc.get(FieldKind::DocumentType).unwrap().value, "P");
        assert_eq!(doc.get(FieldKind::IssuingState).unwrap().value, "UTO");
        assert_eq!(doc.get(FieldKind::Surname).unwrap().value, "ERIKSSON");
        assert_eq!(doc.get(FieldKind::GivenName).unwrap().value, "ANNA MARIA");
        assert_eq!(doc.get(FieldKind::DocumentNumber).unwrap().value, "L898902C3");
        assert_eq!(doc.get(FieldKind::Nationality).unwrap().value, "UTO");
        assert_eq!(doc.get(FieldKind::Sex).unwrap().value, "F");
        assert_eq!(doc.get(FieldKind::DateOfBirth).unwrap().value, "1974-08-12");
        assert_eq!(doc.get(FieldKind::DateOfExpiry).unwrap().value, "2012-04-15");
        assert!(doc.checksums_valid());
    }

    #[test]
    fn parses_the_specimen_id_card() {
        let lines = [
            "I<UTOD231458907<<<<<<<<<<<<<<<",
            "7408122F1204159UTO<<<<<<<<<<<6",
            "ERIKSSON<<ANNA<MARIA<<<<<<<<<<",
        ];
        let doc = parse(&lines).expect("specimen parses");

        assert_eq!(doc.get(FieldKind::DocumentNumber).unwrap().value, "D23145890");
        assert_eq!(doc.get(FieldKind::Surname).unwrap().value, "ERIKSSON");
        assert_eq!(doc.get(FieldKind::GivenName).unwrap().value, "ANNA MARIA");
        assert_eq!(doc.get(FieldKind::DateOfBirth).unwrap().value, "1974-08-12");
        assert_eq!(doc.get(FieldKind::Nationality).unwrap().value, "UTO");
        assert!(doc.checksums_valid());
    }

    #[test]
    fn parses_the_specimen_td2() {
        let lines =
            ["I<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<", "D231458907UTO7408122F1204159<<<<<<<6"];
        let doc = parse(&lines).expect("specimen parses");
        assert_eq!(doc.get(FieldKind::DocumentNumber).unwrap().value, "D23145890");
        assert_eq!(doc.get(FieldKind::Surname).unwrap().value, "ERIKSSON");
        assert!(doc.checksums_valid());
    }

    #[test]
    fn a_corrupted_check_digit_is_reported_not_hidden() {
        let lines = [
            "P<UTOERIKSSON<<ANNA<MARIA<<<<<<<<<<<<<<<<<<<",
            // document-number check digit changed from 6 to 5
            "L898902C35UTO7408122F1204159ZE184226B<<<<<10",
        ];
        let doc = parse(&lines).expect("still parses");
        assert_eq!(doc.get(FieldKind::DocumentNumber).unwrap().checksum_ok, Some(false));
        assert!(!doc.checksums_valid());
    }

    #[test]
    fn format_detection() {
        assert_eq!(
            MrzFormat::detect(&["x".repeat(44).as_str(), "y".repeat(44).as_str()]),
            Some(MrzFormat::Td3)
        );
        assert_eq!(MrzFormat::detect(&["x"; 2]), None);
    }

    #[test]
    fn wrong_line_length_is_an_error() {
        let lines = ["P<UTO", "L898902C36UTO7408122F1204159ZE184226B<<<<<10"];
        assert!(parse(&lines).is_err());
    }

    #[test]
    fn row_roles_and_ascii_are_required_before_field_slicing() {
        let rows = [
            "I<UTOD231458907<<<<<<<<<<<<<<<",
            "7408122F1204159UTO<<<<<<<<<<<6",
            "ERIKSSON<<ANNA<MARIA<<<<<<<<<<",
        ];
        assert!(parse(&[rows[1], rows[2], rows[0]]).is_err());
        assert!(parse(&[rows[0], rows[1], rows[0]]).is_err());
        let unicode = format!("é{}", "<".repeat(28));
        assert!(parse(&[&unicode, rows[1], rows[2]]).is_err());
        assert!(parse(&[rows[0], "7413322F1204159UTO<<<<<<<<<<<6", rows[2]]).is_err());
        assert!(parse(&[rows[0], "740812<F1204159UTO<<<<<<<<<<<6", rows[2]]).is_err());
    }

    #[test]
    fn a_composite_check_failure_is_reported() {
        let doc = parse(&[
            "I<UTOD231458907<<<<<<<<<<<<<<<",
            "7408122F1204159UTO<<<<<<<<<<<7",
            "ERIKSSON<<ANNA<MARIA<<<<<<<<<<",
        ])
        .unwrap();
        assert_eq!(doc.get(FieldKind::MrzLine).unwrap().checksum_ok, Some(false));
        assert!(!doc.checksums_valid());
    }

    #[test]
    fn birth_dates_use_the_sliding_century_window() {
        assert_eq!(iso_date("740812", 30).unwrap(), "1974-08-12");
        assert_eq!(iso_date("050812", 30).unwrap(), "2005-08-12");
    }
}
