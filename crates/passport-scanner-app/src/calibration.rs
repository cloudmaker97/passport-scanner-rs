//! Finding the unit's flat-field calibration on this machine.
//!
//! The vendor's calibration program writes it to a shared folder named after
//! the USB identifiers, so a second scanner of a different model gets its own.
//! The vendor files remain untouched. A validated white-sheet measurement
//! can be written to this application's separate versioned file.

use passport_scanner_device::CaptureProfile;
use passport_scanner_imaging::Shading;
use std::path::{Path, PathBuf};

/// Where the vendor's tools keep per-unit calibration.
pub const ROOT: &str = r"C:\Users\Public\Documents\Plustek";

/// Calibration paired with the factory General capture profile.
///
/// The sensor uses the first 29-entry source table, General AWB gains at
/// EEPROM 0x3FD0..0x3FE4, and exposure at 0x3FFC. These settings and this
/// spatial gain map must change together: the ID profile uses different
/// exposure, AWB and lens-shading coefficients. Mixing profiles produces
/// clipping, colour casts and gradients that a later tone curve cannot fix.
pub const DOCUMENT_FILE: &str = "Calibration_General.shd";

/// Calibration for the separate factory ID profile, not selected by the
/// current General-profile capture path.
pub const IDENTITY_FILE: &str = "Calibration_General_ID.shd";

/// What `passport-scanner-calibrate` writes.
///
/// A name of its own, so the vendor's files are never written over: removing
/// this one goes straight back to them. It is also read first, so a
/// calibration measured here beats one measured by other software under
/// settings this one does not use.
// Version 1 stored top-down rows, unlike the vendor's bottom-up DIB layout.
// Do not silently reinterpret an old calibration using the corrected reader.
// Leave that file untouched and fall back to the vendor until recalibration.
pub const OWN_FILE: &str = "Calibration_PassportScanner_v2.shd";

/// Calibration files accepted in preference order for the General profile.
pub const USABLE: [&str; 2] = [OWN_FILE, DOCUMENT_FILE];

/// Calibration loaded for one capture profile. Report this active source,
/// rather than looking on disk again after the scanner has been configured.
#[derive(Debug)]
pub struct Loaded {
    pub profile: CaptureProfile,
    pub source: Option<&'static str>,
    pub shading: Option<Shading>,
}

/// Load a matched profile/calibration pair. Identity requires its own vendor
/// file and never falls back to General or a General white-sheet measurement.
pub fn load_for_profile(
    vendor: u16,
    product: u16,
    profile: CaptureProfile,
) -> passport_scanner_core::Result<Loaded> {
    load_from_folder(&folder(vendor, product), profile)
}

fn load_from_folder(dir: &Path, profile: CaptureProfile) -> passport_scanner_core::Result<Loaded> {
    let names: &[&'static str] = match profile {
        CaptureProfile::General => &USABLE,
        CaptureProfile::Identity => &[IDENTITY_FILE],
    };
    for &name in names {
        let path = dir.join(name);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        let shading = Shading::parse(&bytes).map_err(|e| {
            passport_scanner_core::Error::Malformed(format!("{}: {e}", path.display()))
        })?;
        return Ok(Loaded { profile, source: Some(name), shading: Some(shading) });
    }
    if profile == CaptureProfile::Identity {
        return Err(passport_scanner_core::Error::Rejected(format!(
            "Identity requires {IDENTITY_FILE}. Select General until the matching calibration is available."
        )));
    }
    Ok(Loaded { profile, source: None, shading: None })
}

/// Shading-only convenience for capture tools that already selected a profile.
pub fn for_profile(
    vendor: u16,
    product: u16,
    profile: CaptureProfile,
) -> passport_scanner_core::Result<Option<Shading>> {
    Ok(load_for_profile(vendor, product, profile)?.shading)
}

/// Own measurements belong to General. Check before capturing, writing or
/// deleting anything when invoked by a profile-aware caller.
pub fn require_general(profile: CaptureProfile) -> passport_scanner_core::Result<()> {
    if profile == CaptureProfile::Identity {
        return Err(passport_scanner_core::Error::Rejected(
            "Switch to General to measure or remove a calibration. Identity uses its matching factory calibration.".into()
        ));
    }
    Ok(())
}

/// The folder a unit's calibration lives in.
#[must_use]
pub fn folder(vendor: u16, product: u16) -> PathBuf {
    Path::new(ROOT).join(format!("VID_{vendor:04X}&PID_{product:04X}"))
}

/// Load the calibration for an identity document, if this machine has one.
///
/// Returns `None` rather than failing: a scanner that has never been
/// calibrated on this machine still scans, just less evenly. A file that is
/// present but unreadable is worth knowing about, so that comes back as an
/// error.
///
/// # Errors
/// Fails only if a file is there and is not a shading file.
pub fn for_documents(vendor: u16, product: u16) -> passport_scanner_core::Result<Option<Shading>> {
    for_profile(vendor, product, CaptureProfile::General)
}

/// What a reference capture has to look like before it is worth measuring.
///
/// A calibration built from something that is not a blank white sheet makes
/// every later scan worse, and it is not obvious from the result that this is
/// what happened. So the capture is checked first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reference {
    /// Fraction of the frame that is dark.
    pub dark: f64,
    /// Fraction that is lit well enough to measure.
    pub lit: f64,
    /// Fraction with at least one channel close to sensor clipping.
    pub clipped: f64,
    /// Fraction of strong local edges, indicating print or an object boundary.
    pub detail: f64,
    /// Mean relative channel spread in the lit area; rejects coloured paper.
    pub chroma: f64,
}

impl Reference {
    /// Above this share of dark pixels, the platen is not covered.
    pub const MAX_DARK: f64 = 0.25;
    /// Below this share of lit pixels, it is not covered corner to corner.
    pub const MIN_LIT: f64 = 0.6;

    /// Measure a capture.
    #[must_use]
    pub fn of(image: &image::RgbImage) -> Self {
        let mut histogram = [0u64; 256];
        let mut clipped = 0u64;
        let mut chroma = 0.0;
        let mut colour_samples = 0u64;
        for p in image.pixels() {
            histogram[passport_scanner_imaging::luma(p.0) as usize] += 1;
            clipped += u64::from(p.0.iter().any(|v| *v >= 250));
            if passport_scanner_imaging::luma(p.0) >= 120 {
                let high = *p.0.iter().max().unwrap();
                let low = *p.0.iter().min().unwrap();
                chroma += f64::from(high - low) / f64::from(high);
                colour_samples += 1;
            }
        }
        let total: u64 = histogram.iter().sum::<u64>().max(1);
        // Measure at a fixed scale so sensor noise is averaged out and the
        // criterion does not change with capture resolution. Illumination
        // varies smoothly; lettering and card edges do not.
        let small = image::imageops::resize(image, 648, 486, image::imageops::FilterType::Triangle);
        let mut detail = 0u64;
        for y in 1..small.height() {
            for x in 1..small.width() {
                let p = small.get_pixel(x, y).0;
                let left = small.get_pixel(x - 1, y).0;
                let above = small.get_pixel(x, y - 1).0;
                if (0..3).any(|c| p[c].abs_diff(left[c]) > 32 || p[c].abs_diff(above[c]) > 32) {
                    detail += 1;
                }
            }
        }
        Self {
            dark: histogram[..64].iter().sum::<u64>() as f64 / total as f64,
            lit: histogram[120..].iter().sum::<u64>() as f64 / total as f64,
            clipped: clipped as f64 / total as f64,
            detail: detail as f64 / f64::from(647 * 485),
            chroma: chroma / colour_samples.max(1) as f64,
        }
    }

    /// Why this capture will not do, if it will not.
    #[must_use]
    pub fn objection(&self) -> Option<String> {
        if self.dark > Self::MAX_DARK {
            return Some(format!(
                "{:.0}% of the frame is dark. Cover the platen with a sheet of plain white paper.",
                self.dark * 100.0
            ));
        }
        if self.lit < Self::MIN_LIT {
            return Some(format!(
                "only {:.0}% of the frame is well lit. Check the sheet covers the platen corner                  to corner.",
                self.lit * 100.0
            ));
        }
        if self.clipped > 0.02 {
            return Some("The reference is overexposed. A clipped white sheet cannot measure the lamp's brightness.".into());
        }
        if self.detail > 0.008 {
            return Some("The reference contains print or strong edges. Use plain, unprinted white paper covering the whole platen.".into());
        }
        if self.chroma > 0.25 {
            return Some("The reference is strongly coloured. Use neutral white paper to avoid calibrating a colour cast into every scan.".into());
        }
        None
    }
}

/// Build a calibration from a capture of a blank white platen, and save it.
///
/// # Errors
/// Refuses, without writing anything, if the capture is not of a blank white
/// sheet covering the platen. Also fails if the file cannot be written.
pub fn measure(
    image: &image::RgbImage,
    vendor: u16,
    product: u16,
) -> passport_scanner_core::Result<Shading> {
    if let Some(objection) = Reference::of(image).objection() {
        return Err(passport_scanner_core::Error::Rejected(objection));
    }
    let shading =
        Shading::from_reference(image, passport_scanner_imaging::flatfield::TARGET_VISIBLE);

    let path = own_file(vendor, product);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Finish writing before replacing a usable calibration. An interrupted
    // write must not leave a truncated file that takes precedence on restart.
    let pending = path.with_extension("shd.pending");
    std::fs::write(&pending, shading.to_bytes())?;
    std::fs::rename(&pending, &path)?;
    Ok(shading)
}

/// Forget a calibration measured here, going back to the vendor's.
///
/// # Errors
/// Fails if the file is there and cannot be removed. Nothing to remove is not
/// an error.
pub fn forget(vendor: u16, product: u16) -> passport_scanner_core::Result<()> {
    match std::fs::remove_file(own_file(vendor, product)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Where a calibration measured by this software goes.
#[must_use]
pub fn own_file(vendor: u16, product: u16) -> PathBuf {
    folder(vendor, product).join(OWN_FILE)
}

/// Which calibration is in use for a unit, for reporting to someone.
#[must_use]
pub fn in_use(vendor: u16, product: u16) -> Option<&'static str> {
    let dir = folder(vendor, product);
    USABLE.into_iter().find(|name| dir.join(name).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_loading_never_substitutes_general_for_identity() {
        let dir = std::env::temp_dir().join(format!(
            "passport-scanner-shading-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let sheet = image::RgbImage::from_pixel(16, 12, image::Rgb([190; 3]));
        let bytes =
            Shading::from_reference(&sheet, passport_scanner_imaging::flatfield::TARGET_VISIBLE)
                .to_bytes();
        std::fs::write(dir.join(DOCUMENT_FILE), &bytes).unwrap();
        assert_eq!(
            load_from_folder(&dir, CaptureProfile::General).unwrap().source,
            Some(DOCUMENT_FILE)
        );
        assert!(load_from_folder(&dir, CaptureProfile::Identity).is_err());
        std::fs::write(dir.join(OWN_FILE), &bytes).unwrap();
        std::fs::write(dir.join(IDENTITY_FILE), &bytes).unwrap();
        assert_eq!(load_from_folder(&dir, CaptureProfile::General).unwrap().source, Some(OWN_FILE));
        assert_eq!(
            load_from_folder(&dir, CaptureProfile::Identity).unwrap().source,
            Some(IDENTITY_FILE)
        );
        std::fs::write(dir.join(IDENTITY_FILE), b"invalid shading").unwrap();
        assert!(load_from_folder(&dir, CaptureProfile::Identity).is_err());
        assert!(require_general(CaptureProfile::Identity).is_err());
        assert!(require_general(CaptureProfile::General).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_unit_folder_is_named_after_its_usb_identifiers() {
        // Upper-case hex, four digits, exactly as the vendor's tools write it.
        assert!(folder(0x07B3, 0x1A05).ends_with("VID_07B3&PID_1A05"));
        assert!(folder(0x0001, 0x000f).ends_with("VID_0001&PID_000F"));
    }

    #[test]
    fn a_calibration_measured_here_goes_beside_the_vendors_not_over_it() {
        let own = own_file(0x07B3, 0x1A05);
        assert!(own.ends_with(OWN_FILE));
        assert_ne!(OWN_FILE, DOCUMENT_FILE);
        assert_ne!(OWN_FILE, IDENTITY_FILE);
        assert_eq!(own.parent(), Some(folder(0x07B3, 0x1A05).as_path()));
    }

    #[test]
    fn a_dark_capture_is_refused_as_a_reference() {
        let dark = Reference { dark: 0.57, lit: 0.37, clipped: 0.0, detail: 0.0, chroma: 0.0 };
        assert!(dark.objection().is_some());
    }

    #[test]
    fn a_partly_covered_platen_is_refused_too() {
        let partial = Reference { dark: 0.1, lit: 0.45, clipped: 0.0, detail: 0.0, chroma: 0.0 };
        assert!(partial.objection().is_some());
    }

    #[test]
    fn a_blank_white_platen_is_accepted() {
        let good = Reference { dark: 0.02, lit: 0.95, clipped: 0.0, detail: 0.0, chroma: 0.0 };
        assert_eq!(good.objection(), None);
    }

    #[test]
    fn the_measurement_of_a_blank_sheet_says_it_is_one() {
        let sheet = image::RgbImage::from_pixel(64, 48, image::Rgb([200, 200, 200]));
        let r = Reference::of(&sheet);
        assert!(r.lit > 0.99 && r.dark < 0.01, "{r:?}");
        assert_eq!(r.objection(), None);
    }

    #[test]
    fn the_measurement_of_an_empty_platen_says_it_is_not() {
        let platen = image::RgbImage::from_pixel(64, 48, image::Rgb([8, 9, 8]));
        assert!(Reference::of(&platen).objection().is_some());
    }

    #[test]
    fn printed_paper_is_not_a_white_reference_despite_being_mostly_lit() {
        let printed = image::RgbImage::from_fn(648, 486, |x, y| {
            image::Rgb(if x > 60 && x < 580 && y % 30 < 3 { [35; 3] } else { [200; 3] })
        });
        let reference = Reference::of(&printed);
        assert!(reference.lit > Reference::MIN_LIT);
        assert!(reference.dark < Reference::MAX_DARK);
        assert!(reference.objection().unwrap().contains("print"));
    }

    #[test]
    fn a_clipped_white_sheet_is_not_a_measurable_reference() {
        let clipped = image::RgbImage::from_pixel(64, 48, image::Rgb([230, 255, 240]));
        assert!(Reference::of(&clipped).objection().unwrap().contains("overexposed"));
    }

    #[test]
    fn smooth_uneven_illumination_with_a_small_lamp_cast_is_accepted() {
        let reference = image::RgbImage::from_fn(648, 486, |x, y| {
            let v = (100 + 100 * (x + y) / (648 + 486)) as u8;
            image::Rgb([v, v + 15, v + 8])
        });
        let measured = Reference::of(&reference);
        assert_eq!(measured.objection(), None, "{measured:?}");
    }

    #[test]
    fn coloured_paper_cannot_define_the_white_balance() {
        let reference = image::RgbImage::from_pixel(648, 486, image::Rgb([150, 200, 100]));
        assert!(Reference::of(&reference).objection().unwrap().contains("coloured"));
    }

    #[test]
    fn nothing_calibrated_is_reported_as_nothing() {
        assert_eq!(in_use(0xFFFF, 0xFFFF), None);
    }

    #[test]
    fn the_two_calibrations_are_not_interchangeable() {
        // Named apart so that picking one is a decision with a reason behind
        // it rather than whichever happened to be found first.
        assert_ne!(DOCUMENT_FILE, IDENTITY_FILE);
    }

    #[test]
    fn an_absent_folder_is_not_an_error() {
        // A scanner nobody has calibrated here is a normal state of affairs.
        assert!(matches!(for_documents(0xFFFF, 0xFFFF), Ok(None)));
    }
}
