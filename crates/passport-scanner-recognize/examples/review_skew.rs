//! Private local skew replay. Reports checks/field equality, never field values.
//! Arguments: baseline white PNG, then variant white PNGs. Each has an
//! `-infrared.png` sibling. Generate fixtures independently of production code.
use passport_scanner_core::{
    Document, DocumentRecognizer, FieldKind, Frame, LightSource, PixelFormat,
};
use passport_scanner_recognize::{Engine, MrzReader, Tesseract};
use std::time::Instant;

#[derive(Debug)]
struct MrzOnly(Tesseract);
impl Engine for MrzOnly {
    fn read(&self, image: &image::GrayImage) -> passport_scanner_core::Result<String> {
        self.0.read(image)
    }
    fn parallel_sources(&self) -> bool {
        true
    }
    // Intentionally disable non-MRZ orientation: this diagnoses MRZ recovery.
}

fn essential_equal(a: &Document, b: &Document) -> bool {
    [
        FieldKind::GivenName,
        FieldKind::Surname,
        FieldKind::DocumentNumber,
        FieldKind::DateOfBirth,
        FieldKind::DateOfExpiry,
        FieldKind::Nationality,
        FieldKind::IssuingState,
        FieldKind::Sex,
    ]
    .iter()
    .all(|kind| a.get(*kind) == b.get(*kind))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let reader = MrzReader::new(MrzOnly(Tesseract::default()));
    let mut baseline = None;
    for (index, path) in std::env::args().skip(1).enumerate() {
        let infrared = path.replace("-white.png", "-infrared.png");
        let frames =
            [(path.as_str(), LightSource::White), (infrared.as_str(), LightSource::Infrared)]
                .into_iter()
                .map(|(path, source)| {
                    Ok(Frame {
                        width: 0,
                        height: 0,
                        format: PixelFormat::Jpeg,
                        source,
                        data: std::fs::read(path)?,
                    })
                })
                .collect::<std::io::Result<Vec<_>>>()?;
        if std::env::var_os("SKEW_STRIP_DIAGNOSTICS").is_some() {
            for (source, filename) in [("white", path.as_str()), ("infrared", infrared.as_str())] {
                let image = image::open(filename)?.to_luma8();
                if let Some(bounds) = passport_scanner_imaging::crop::bounds(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                    110,
                ) {
                    let expanded =
                        bounds.grown((bounds.w / 100).max(2), image.width(), image.height());
                    let roi = passport_scanner_recognize::locate::zone_strip_for_skew(expanded);
                    let band =
                        image::imageops::crop_imm(&image, roi.x, roi.y, roi.w, roi.h).to_image();
                    let started = Instant::now();
                    let estimate = passport_scanner_recognize::deskew::estimate(&band);
                    println!(
                        "fixture={index} source={source} estimate={estimate:?} elapsed_ms={} roi={roi:?}",
                        started.elapsed().as_millis()
                    );
                }
            }
            continue;
        }
        let start = Instant::now();
        let document = reader.recognize(&frames)?;
        let valid = !document.fields.is_empty() && document.checksums_valid();
        let same = valid && baseline.as_ref().is_some_and(|b| essential_equal(b, &document));
        let image = image::open(&path)?.to_luma8();
        let bounds = passport_scanner_imaging::crop::bounds(
            image.as_raw(),
            image.width(),
            image.height(),
            110,
        );
        let strip = bounds.map(passport_scanner_recognize::locate::zone_strip);
        println!(
            "fixture={index} valid={valid} same_essential={same} elapsed_ms={} dimensions={}x{} strip={strip:?}",
            start.elapsed().as_millis(),
            image.width(),
            image.height()
        );
        if index == 0 {
            if !valid {
                return Err("baseline must verify before comparison".into());
            }
            baseline = Some(document);
        }
    }
    Ok(())
}
