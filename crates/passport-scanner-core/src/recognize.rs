//! Extracting data from a captured document.

use crate::{error::Result, frame::Frame};
use std::collections::BTreeMap;

/// What a field means, independent of which recogniser produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum FieldKind {
    /// Given name(s).
    GivenName,
    /// Family name(s).
    Surname,
    /// Document number.
    DocumentNumber,
    /// Nationality, ISO 3166-1 alpha-3.
    Nationality,
    /// Issuing state, ISO 3166-1 alpha-3.
    IssuingState,
    /// Date of birth, ISO 8601.
    DateOfBirth,
    /// Expiry date, ISO 8601.
    DateOfExpiry,
    /// Sex as printed.
    Sex,
    /// Document type code.
    DocumentType,
    /// Optional data, as carried in the machine-readable zone.
    OptionalData,
    /// A raw machine-readable-zone line.
    MrzLine,
}

/// One extracted value.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// What this value is.
    pub kind: FieldKind,
    /// The value itself, already normalised where the format allows it.
    pub value: String,
    /// `Some(true)` or `Some(false)` when the format carries a check digit that could be
    /// verified; `None` when the field has no checksum.
    pub checksum_ok: Option<bool>,
}

impl Field {
    /// A field whose format carries no checksum.
    #[must_use]
    pub fn plain(kind: FieldKind, value: impl Into<String>) -> Self {
        Self { kind, value: value.into(), checksum_ok: None }
    }

    /// A field read from a checksummed format.
    #[must_use]
    pub fn checked(kind: FieldKind, value: impl Into<String>, ok: bool) -> Self {
        Self { kind, value: value.into(), checksum_ok: Some(ok) }
    }
}

/// Everything one recogniser extracted from one document.
#[derive(Debug, Clone, Default)]
pub struct Document {
    /// Extracted values, keyed so later recognisers can refine earlier results.
    pub fields: BTreeMap<FieldKind, Field>,
}

impl Document {
    /// Look up a value.
    #[must_use]
    pub fn get(&self, kind: FieldKind) -> Option<&Field> {
        self.fields.get(&kind)
    }

    /// Insert or replace a value.
    pub fn insert(&mut self, field: Field) {
        self.fields.insert(field.kind, field);
    }

    /// Did every checksummed field verify?
    ///
    /// Fields without a checksum do not count against the result.
    #[must_use]
    pub fn checksums_valid(&self) -> bool {
        self.fields.values().all(|f| f.checksum_ok.unwrap_or(true))
    }

    /// Merge `other` into `self`, letting `other` win on conflicts.
    ///
    /// This is how a chain of recognisers accumulates a result: a barcode or chip read
    /// can override a lower-confidence OCR guess for the same field.
    pub fn merge(&mut self, other: Document) {
        self.fields.extend(other.fields);
    }
}

/// Something that extracts data from captured images.
///
/// Implementations are independent and composable: an MRZ reader, an OCR engine, a
/// PDF417 barcode decoder and an NFC chip reader all satisfy this trait and can be run
/// in sequence over the same capture.
pub trait DocumentRecognizer: Send {
    /// Human-readable name, used in logs and in the UI.
    fn name(&self) -> &str;

    /// Attempt extraction. Returning an empty [`Document`] means "nothing found here",
    /// which is not an error.
    fn recognize(&self, frames: &[Frame]) -> Result<Document>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_recognizer_wins_on_merge() {
        let mut a = Document::default();
        a.insert(Field::plain(FieldKind::Surname, "MUELLER"));
        let mut b = Document::default();
        b.insert(Field::checked(FieldKind::Surname, "MÜLLER", true));

        a.merge(b);
        assert_eq!(a.get(FieldKind::Surname).unwrap().value, "MÜLLER");
    }

    #[test]
    fn unchecked_fields_do_not_invalidate() {
        let mut doc = Document::default();
        doc.insert(Field::plain(FieldKind::OptionalData, "whatever"));
        doc.insert(Field::checked(FieldKind::DocumentNumber, "L01X00T47", true));
        assert!(doc.checksums_valid());
    }

    #[test]
    fn a_failed_checksum_invalidates() {
        let mut doc = Document::default();
        doc.insert(Field::checked(FieldKind::DocumentNumber, "L01X00T47", false));
        assert!(!doc.checksums_valid());
    }
}
