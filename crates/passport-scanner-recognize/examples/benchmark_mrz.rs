//! Local performance measurements. Never prints OCR text or personal fields.
use passport_scanner_core::{DocumentRecognizer, Frame, LightSource, PixelFormat};
use passport_scanner_recognize::{Engine, MrzReader, Tesseract};
use std::time::Instant;

#[derive(Debug)]
struct Timed(Tesseract);
impl Engine for Timed {
    fn orientation(&self, image: &image::GrayImage) -> passport_scanner_core::Result<Option<i16>> {
        let start = Instant::now();
        let result = self.0.orientation(image);
        println!(
            "orientation elapsed_ms={} correction={:?}",
            start.elapsed().as_millis(),
            result.as_ref().ok().copied().flatten()
        );
        result
    }
    fn parallel_sources(&self) -> bool {
        self.0.parallel_sources() && std::env::var_os("MRZ_BENCH_SEQUENTIAL").is_none()
    }
    fn read(&self, image: &image::GrayImage) -> passport_scanner_core::Result<String> {
        let start = Instant::now();
        let result = self.0.read(image);
        println!(
            "ocr width={} height={} elapsed_ms={}",
            image.width(),
            image.height(),
            start.elapsed().as_millis()
        );
        result
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let reader = MrzReader::new(Timed(Tesseract::default()));
    let mut frames = Vec::new();
    for (index, path) in std::env::args().skip(1).enumerate() {
        let frame = Frame {
            width: 0,
            height: 0,
            format: PixelFormat::Jpeg,
            source: if index == 0 { LightSource::White } else { LightSource::Infrared },
            data: std::fs::read(path)?,
        };
        println!("frame index={index} source={:?}", frame.source);
        let start = Instant::now();
        let (reading, angle) = reader.read_frame_oriented(&frame)?;
        let valid = reading.lines.as_ref().is_some_and(|lines| {
            passport_scanner_mrz::parse(&lines.iter().map(String::as_str).collect::<Vec<_>>())
                .is_ok_and(|d| d.checksums_valid())
        });
        println!(
            "frame elapsed_ms={} correction={angle} valid={valid}",
            start.elapsed().as_millis()
        );
        frames.push(frame);
    }
    let start = Instant::now();
    let document = reader.recognize(&frames)?;
    println!(
        "combined elapsed_ms={} fields_present={} valid={}",
        start.elapsed().as_millis(),
        !document.fields.is_empty(),
        !document.fields.is_empty() && document.checksums_valid()
    );
    Ok(())
}
