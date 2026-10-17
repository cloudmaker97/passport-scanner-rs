//! The wire contract between the Passport Scanner core and any front end.
//!
//! The desktop application, a browser talking to a local daemon, and an
//! integrator's own code all speak these same messages. Nothing in here depends
//! on a transport, so the identical [`Command`] can arrive through the native
//! Iced host or an HTTP request from the local web interface.
//!
//! Keeping the front end on this contract rather than on host-native calls is
//! what lets one interface serve the desktop app and the web UI, and what makes
//! the SDK's surface the same surface the interface itself uses.
//!
//! # Compatibility
//!
//! [`VERSION`] is sent in [`Event::Hello`] when a session opens. Enums are
//! `#[non_exhaustive]` and carry a `serde` tag, so a front end that meets an
//! unfamiliar variant can report it rather than mis-parse neighbouring fields.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Contract version, reported at the start of every session.
pub const VERSION: u32 = 1;

/// Which illumination an image was taken under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Visible white light.
    White,
    /// Infrared.
    Infrared,
    /// Ultraviolet.
    Ultraviolet,
}

impl From<passport_scanner_core::LightSource> for Source {
    fn from(value: passport_scanner_core::LightSource) -> Self {
        match value {
            passport_scanner_core::LightSource::White => Self::White,
            passport_scanner_core::LightSource::Infrared => Self::Infrared,
            passport_scanner_core::LightSource::Ultraviolet => Self::Ultraviolet,
        }
    }
}

/// The scanner, as far as a front end needs to know.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Marketing name, e.g. `"X-Mini 2000"`.
    pub model: String,
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
    /// Serial number, when the unit reports one.
    pub serial: Option<String>,
    /// Illumination this unit is fitted with.
    pub sources: Vec<Source>,
}

/// One captured image.
///
/// Pixels never travel inside a message. `url` points at them - a `data:` URL for
/// something small, otherwise an address the host serves, so a large frame does
/// not have to be base64-encoded through the channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    /// Illumination this was taken under.
    pub source: Source,
    /// Where the bytes are.
    pub url: String,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// What a field means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// Optional data from the machine-readable zone.
    OptionalData,
}

/// One extracted value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// What this value is.
    pub kind: FieldKind,
    /// The value, normalised where the format allows it.
    pub value: String,
    /// Whether the format's check digit verified. `None` when the field carries none.
    pub check: Option<bool>,
    /// Where in the zone this was read: line index, then the half-open character
    /// range. This is what lets an interface show a value's provenance.
    pub span: Option<Span>,
}

/// A character range within one line of the machine-readable zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    /// Zero-based line index.
    pub line: u8,
    /// First character, inclusive.
    pub start: u8,
    /// Last character, exclusive.
    pub end: u8,
}

/// Which face of a document a capture shows.
///
/// An identity card has two, and only one of them carries the
/// machine-readable zone, so a single capture is half a document. Which face
/// is which is decided by the order they were taken in: the first capture of
/// a document is its front. That is a convention rather than a measurement -
/// nothing on a card says which side it is - but it matches how anyone
/// actually puts one on a platen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Side {
    /// The first face captured.
    Front,
    /// The second.
    Back,
}

/// One completed scan: one face of one document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    /// Stable identifier, unique within a session.
    pub id: String,
    /// The document this is a face of. Two scans sharing one are two faces of
    /// the same physical card.
    pub document: String,
    /// Which face.
    pub side: Side,
    /// When the scan completed, ISO 8601.
    pub at: String,
    /// Detected zone format, e.g. `"TD1"`. `None` if no zone was found.
    pub format: Option<String>,
    /// Recognized fixed-width MRZ grid, after conservative OCR filler recovery.
    pub mrz: Vec<String>,
    /// Composite MRZ checksum, including optional areas; unknown is not verified.
    #[serde(default)]
    pub mrz_check: Option<bool>,
    /// What was extracted from it.
    pub fields: Vec<Field>,
    /// The captured images.
    pub images: Vec<Image>,
}

impl Scan {
    /// How many fields carried a check digit, and how many of those verified.
    #[must_use]
    pub fn check_tally(&self) -> (usize, usize) {
        let checked: Vec<bool> = self.fields.iter().filter_map(|f| f.check).collect();
        (checked.iter().filter(|ok| **ok).count(), checked.len())
    }

    /// Were field checks and the complete MRZ composite check verified?
    #[must_use]
    pub fn checks_passed(&self) -> bool {
        let (passed, total) = self.check_tally();
        total > 0 && passed == total && self.mrz_check == Some(true) && !self.mrz.is_empty()
    }
}

/// How far along a scan is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Stage {
    /// Waiting for a document to be placed.
    AwaitingDocument,
    /// Illumination set, waiting for the image to settle.
    Settling,
    /// Taking a frame.
    Capturing,
    /// Correcting the captured image.
    Processing,
    /// Reading the document.
    Recognising,
}

/// Sent by a front end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "camelCase")]
#[non_exhaustive]
pub enum Command {
    /// Ask for the current device and scan history. Answered with [`Event::State`].
    GetState,
    /// Begin a scan now, without waiting for the document sensors.
    StartScan,
    /// Atomically choose a document and side and capture it. None starts a new document.
    CaptureSide { side: Side, document: Option<String> },
    /// Keep a scan and hand it to whatever consumes results.
    Accept {
        /// Which scan.
        id: String,
    },
    /// Throw a scan away.
    Discard {
        /// Which scan.
        id: String,
    },
    /// Bring a scan from the history to the front.
    SelectScan {
        /// Which scan.
        id: String,
    },
    /// Turn scanning on placement on or off.
    SetAutoScan {
        /// Whether to scan as soon as a document is detected.
        enabled: bool,
    },
    /// Close the document being assembled, so the next scan starts a new one.
    ///
    /// Needed for a document with only one face worth keeping: without it the
    /// next capture would be taken for that document's second face.
    FinishDocument,
    /// Measure how this unit lights its platen, and keep the result.
    ///
    /// Needs a sheet of plain white paper covering the platen: that sheet is
    /// the reference the correction is built from. Refused, without changing
    /// anything, if what the scanner sees is not one.
    Calibrate,
    /// Discard a calibration measured here, going back to the vendor's.
    ForgetCalibration,
    /// Change a setting.
    SetSetting {
        /// Setting name.
        key: String,
        /// New value.
        value: String,
    },
}

/// Sent by the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
#[non_exhaustive]
pub enum Event {
    /// First message of a session.
    Hello {
        /// Contract version the core speaks.
        version: u32,
    },
    /// A full picture, in answer to [`Command::GetState`].
    State {
        /// The attached scanner, if any.
        device: Option<Device>,
        /// Completed scans, newest first.
        scans: Vec<Scan>,
        /// Which scan the interface should show.
        selected: Option<String>,
    },
    /// The history changed without any new capture: a scan was selected, a
    /// document discarded or closed.
    ///
    /// Carries only identifiers. Every scan it names has already been sent,
    /// with its images, in a [`Event::State`] or [`Event::ScanCompleted`], so
    /// sending the pixels again for a click would be megabytes for nothing. A
    /// front end that does not know one of them asks with
    /// [`Command::GetState`].
    History {
        /// The scans still kept, newest first.
        scans: Vec<String>,
        /// Which scan the interface should show.
        selected: Option<String>,
    },
    /// A scanner was attached or detached.
    DeviceChanged {
        /// The scanner now attached, or `None`.
        device: Option<Device>,
    },
    /// A document was placed or removed.
    DocumentPresence {
        /// Whether anything is on the platen.
        present: bool,
    },
    /// A setting changed, or is being reported for the first time.
    SettingsChanged {
        /// Setting names to their values, as text.
        settings: std::collections::BTreeMap<String, String>,
    },
    /// The calibration in use changed, or is being reported.
    CalibrationChanged {
        /// Which file is in use, or `None` for no calibration at all.
        source: Option<String>,
        /// What it says about the platen, for someone deciding whether to
        /// trust it: the range of white the frame reads across its width.
        range: Option<String>,
    },
    /// Scanning on placement was turned on or off.
    AutoScanChanged {
        /// Whether a scan now starts as soon as a document is detected.
        enabled: bool,
    },
    /// A scan is under way.
    ScanProgress {
        /// What the device is doing.
        stage: Stage,
        /// Which illumination, when the stage involves one.
        source: Option<Source>,
    },
    /// A scan finished.
    ScanCompleted {
        /// The result.
        scan: Box<Scan>,
    },
    /// Something failed.
    Failed {
        /// Short machine-readable code, e.g. `"deviceNotFound"`.
        code: String,
        /// What went wrong, and what to do about it.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(kind: FieldKind, value: &str, check: Option<bool>) -> Field {
        Field { kind, value: value.to_owned(), check, span: None }
    }

    fn scan() -> Scan {
        Scan {
            id: "s1".to_owned(),
            document: "d1".to_owned(),
            side: Side::Front,
            at: "2026-10-16T14:32:00Z".to_owned(),
            format: Some("TD1".to_owned()),
            mrz: vec!["I<UTOD231458907<<<<<<<<<<<<<<<".to_owned()],
            mrz_check: Some(true),
            fields: vec![
                field(FieldKind::Surname, "ERIKSSON", None),
                field(FieldKind::DocumentNumber, "D23145890", Some(true)),
                field(FieldKind::DateOfBirth, "1974-08-12", Some(true)),
            ],
            images: vec![],
        }
    }

    #[test]
    fn commands_are_tagged_so_a_reader_can_dispatch_on_one_key() {
        let json = serde_json::to_string(&Command::Accept { id: "s1".into() }).unwrap();
        assert_eq!(json, r#"{"command":"accept","id":"s1"}"#);
    }

    #[test]
    fn commands_round_trip() {
        for c in [
            Command::GetState,
            Command::StartScan,
            Command::SetAutoScan { enabled: true },
            Command::FinishDocument,
            Command::Discard { id: "s2".into() },
            Command::SetSetting { key: "locale".into(), value: "de".into() },
        ] {
            let json = serde_json::to_string(&c).unwrap();
            let back: Command = serde_json::from_str(&json).unwrap();
            assert_eq!(c, back, "{json}");
        }
    }

    #[test]
    fn events_round_trip() {
        let e = Event::ScanCompleted { scan: Box::new(scan()) };
        let json = serde_json::to_string(&e).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn history_carries_identifiers_only() {
        let e =
            Event::History { scans: vec!["s2".into(), "s1".into()], selected: Some("s2".into()) };
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, r#"{"event":"history","scans":["s2","s1"],"selected":"s2"}"#);
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), e);
    }

    #[test]
    fn field_names_cross_the_wire_in_camel_case() {
        let json = serde_json::to_string(&scan()).unwrap();
        assert!(json.contains(r#""kind":"documentNumber""#), "{json}");
        assert!(json.contains(r#""kind":"dateOfBirth""#), "{json}");
    }

    #[test]
    fn tally_counts_only_fields_that_carry_a_check_digit() {
        let s = scan();
        assert_eq!(s.check_tally(), (2, 2));
        assert!(s.checks_passed());
    }

    #[test]
    fn one_bad_check_digit_fails_the_scan() {
        let mut s = scan();
        s.fields[1].check = Some(false);
        assert_eq!(s.check_tally(), (1, 2));
        assert!(!s.checks_passed());
    }

    #[test]
    fn a_scan_with_no_check_digits_at_all_is_unverified() {
        let mut s = scan();
        s.fields.iter_mut().for_each(|f| f.check = None);
        assert_eq!(s.check_tally(), (0, 0));
        assert!(!s.checks_passed());
    }

    #[test]
    fn spans_survive_the_wire_so_provenance_is_not_lost() {
        let mut s = scan();
        s.fields[0].span = Some(Span { line: 2, start: 0, end: 8 });
        let back: Scan = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.fields[0].span, Some(Span { line: 2, start: 0, end: 8 }));
    }

    #[test]
    fn unknown_or_failed_composite_cannot_be_verified_by_field_checks() {
        let mut s = scan();
        for composite in [None, Some(false)] {
            s.mrz_check = composite;
            assert!(!s.checks_passed());
        }
    }
}
