//! Putting the pieces together: a transport, a camera, and a scan.
//!
//! Everything below the seams stays swappable - this crate only decides which
//! implementations to use on which platform, and what a complete scan consists
//! of.

#![forbid(unsafe_code)]

pub mod calibration;
pub mod desktop_host;
pub mod export;
pub mod local_http;
pub mod mcp;
pub mod orientation;
pub mod pages;
pub mod placement;
pub mod preferences;
pub mod service;

use passport_scanner_core::{Error, Frame, Result};

// Opening a scanner needs a transport, which only Windows has so far.
#[cfg(windows)]
use passport_scanner_capture::CameraSource;
#[cfg(windows)]
use passport_scanner_core::LightSource;
#[cfg(windows)]
use passport_scanner_device::{Model, Scanner, protocol::VENDOR_ID};

/// The capture resolution to ask the camera for.
///
/// The reference unit is a 5 MP sensor; the nearest format it offers is taken.
pub const NATIVE: (u32, u32) = (2592, 1944);

#[cfg(windows)]
type Transport = passport_scanner_transport_ks::KsTransport;

/// A scanner opened on this machine.
#[cfg(windows)]
pub type OpenScanner = Scanner<Transport, CameraSource>;

/// Find a scanner, open both its channels, and load its calibration.
///
/// # Errors
/// [`Error::DeviceNotFound`] when nothing is attached, or a platform error if a
/// channel cannot be opened - most often because another program holds the
/// camera.
#[cfg(windows)]
pub fn open() -> Result<(OpenScanner, Model)> {
    let (transport, info) = Transport::open_first(VENDOR_ID)?;
    let model = Model::from_product_id(info.product)
        .ok_or_else(|| Error::Unsupported(format!("unknown product id {:#06x}", info.product)))?;

    // The camera presents itself by name, not by USB id, so it is matched on
    // the scanner's own description.
    let camera = CameraSource::open_named("scanner", NATIVE.0, NATIVE.1)
        .or_else(|_| CameraSource::open(0, NATIVE.0, NATIVE.1))?;

    let mut scanner = Scanner::new(transport, camera);
    // Timings and image controls come from the shipped driver's own
    // configuration for this model where it is installed, so they are the
    // numbers the device was built around rather than this code's guesses.
    scanner.apply_settings(&passport_scanner_device::settings::Settings::load(
        VENDOR_ID,
        info.product,
    ));
    scanner.initialise()?;
    Ok((scanner, model))
}

/// Open a scanner with a matching capture profile and loaded calibration.
/// Identity is refused when its calibration is missing or invalid.
#[cfg(windows)]
pub fn open_with_profile(
    profile: passport_scanner_device::CaptureProfile,
) -> Result<(OpenScanner, Model, calibration::Loaded)> {
    let (mut scanner, model) = open()?;
    let loaded = select_profile(&mut scanner, model, profile)?;
    Ok((scanner, model, loaded))
}

/// Open a named selection, including its bounded factory exposure compensation.
#[cfg(windows)]
pub fn open_with_selection(
    selection: preferences::CaptureSelection,
) -> Result<(OpenScanner, Model, calibration::Loaded)> {
    let (mut scanner, model) = open()?;
    let loaded = select_capture(&mut scanner, model, selection)?;
    Ok((scanner, model, loaded))
}

/// Load shading and validate a fresh factory profile before committing the
/// sensor settings. Repeated selection never compounds exposure compensation.
#[cfg(windows)]
pub fn select_capture(
    scanner: &mut OpenScanner,
    model: Model,
    selection: preferences::CaptureSelection,
) -> Result<calibration::Loaded> {
    let loaded = calibration::load_for_profile(VENDOR_ID, model.product_id(), selection.profile())?;
    scanner.set_capture_profile_with_exposure_percent(
        selection.profile(),
        selection.exposure_percent(),
    )?;
    Ok(loaded)
}

/// Validate shading before changing the scanner. The device API leaves its
/// previous profile intact on validation failure; callers swap the returned
/// calibration into their active state before allowing another capture.
#[cfg(windows)]
pub fn select_profile(
    scanner: &mut OpenScanner,
    model: Model,
    profile: passport_scanner_device::CaptureProfile,
) -> Result<calibration::Loaded> {
    let loaded = calibration::load_for_profile(VENDOR_ID, model.product_id(), profile)?;
    scanner.set_capture_profile(profile)?;
    Ok(loaded)
}

/// Make a set of captures readable.
///
/// The visible-light calibration corrects spatial illumination and colour
/// using the sensor settings under which it was measured. It is applied to
/// the full sensor image before cropping; calibrated output then receives
/// modest sharpening without a scene-dependent RGB stretch.
///
/// `shading` is the calibration, where this machine has one; see
/// [`calibration`]. Without it the fallback can estimate global colour and
/// contrast, but cannot reproduce a measured spatial calibration.
///
/// It is a seam, not a fixed step: a caller that wants the sensor data
/// untouched runs [`passport_scanner_imaging::Enhance::none`] instead.
///
/// # Errors
/// Fails if a frame cannot be decoded or re-encoded. One bad frame stops the
/// run rather than leaving a half-processed set behind.
pub fn enhance(
    frames: &mut [Frame],
    shading: Option<&passport_scanner_imaging::Shading>,
) -> Result<()> {
    use passport_scanner_core::ImageProcessor;

    // One pipeline per source: the correction a capture needs depends on which
    // lamp took it.
    let pipeline =
        |source| passport_scanner_imaging::Enhance::for_documents_with(shading.cloned(), source);

    // A frame is five megapixels and there is one per lamp, so the stages are
    // worth running side by side: the frames are independent and the work is
    // the same size on each.
    std::thread::scope(|scope| {
        let runs: Vec<_> = frames
            .iter_mut()
            .map(|frame| scope.spawn(|| pipeline(frame.source).process(frame)))
            .collect();
        for run in runs {
            run.join().map_err(|_| Error::Rejected("an image stage panicked".to_owned()))??;
        }
        Ok(())
    })
}

/// Re-encode frames without changing them.
///
/// For a caller that wants a frame written out in the same codec and quality
/// the pipeline uses, so that what it saves is comparable with what the
/// pipeline produces rather than differing by the encoder.
///
/// # Errors
/// Fails if a frame cannot be decoded or re-encoded.
pub fn enhance_none(frames: &mut [Frame]) -> Result<()> {
    use passport_scanner_core::ImageProcessor;
    let passthrough = passport_scanner_imaging::Enhance {
        flat_field: None,
        crop: false,
        stages: vec![Box::new(ReEncode)],
    };
    for frame in frames {
        passthrough.process(frame)?;
    }
    Ok(())
}

/// A stage that changes nothing, so a frame goes through the encoder anyway.
#[derive(Debug)]
struct ReEncode;

impl passport_scanner_imaging::Stage for ReEncode {
    fn name(&self) -> &str {
        "re-encode"
    }
    fn apply(&self, _image: &mut image::RgbImage) {}
}

/// Take one capture under every fitted source on a scanner that is already
/// open, and make the frames readable.
///
/// The service holds one scanner for its whole run - it has to, because
/// watching the platen means keeping the device - so it needs the body of a
/// scan without the opening and closing around it.
///
/// # Errors
/// Fails if a capture does not complete, or a frame cannot be processed.
#[cfg(windows)]
pub fn scan_open(
    scanner: &mut OpenScanner,
    model: Model,
    shading: Option<&passport_scanner_imaging::Shading>,
) -> Result<Vec<Frame>> {
    let sources: Vec<LightSource> = model.descriptor().sources;
    let mut frames = scanner.scan(&sources)?;
    enhance(&mut frames, shading)?;
    Ok(frames)
}

/// Open a scanner, take one capture under every fitted source, and hand back
/// the frames, made readable.
///
/// # Errors
/// Fails if the device cannot be opened or a capture does not complete.
#[cfg(windows)]
pub fn scan_once() -> Result<(Model, Vec<Frame>)> {
    let (mut scanner, model) = open()?;
    let shading = calibration::for_documents(VENDOR_ID, model.product_id())?;
    let frames = scan_open(&mut scanner, model, shading.as_ref())?;
    Ok((model, frames))
}

/// The file extension a frame should be written with.
#[must_use]
pub fn extension(frame: &Frame) -> &'static str {
    match frame.format {
        passport_scanner_core::PixelFormat::Jpeg => "jpg",
        _ => "bin",
    }
}

/// Read a document out of a set of captured frames.
///
/// Returns an empty [`passport_scanner_core::Document`] when there is nothing to
/// read - a driving licence without a machine-readable zone is a perfectly
/// good scan, just not a readable one.
///
/// # Errors
/// Fails only if the recogniser itself cannot run.
pub fn read_document(frames: &[Frame]) -> Result<passport_scanner_core::Document> {
    use passport_scanner_core::DocumentRecognizer;
    let reader = passport_scanner_recognize::MrzReader::new(
        passport_scanner_recognize::Tesseract::default(),
    );
    reader.recognize(frames)
}

/// Recognize a document and retain the evidence-backed display orientation.
pub fn read_document_oriented(frames: &[Frame]) -> Result<passport_scanner_recognize::Recognition> {
    let reader = passport_scanner_recognize::MrzReader::new(
        passport_scanner_recognize::Tesseract::default(),
    );
    reader.recognize_oriented(frames)
}
