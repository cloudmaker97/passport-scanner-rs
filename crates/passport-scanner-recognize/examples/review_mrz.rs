//! Offline MRZ diagnostics. Prints only structure/check results, never field values.
use passport_scanner_core::{Frame, LightSource, PixelFormat};
use passport_scanner_recognize::{Engine, MrzReader, Tesseract};

#[derive(Debug)]
struct Shapes(Tesseract);
impl Engine for Shapes {
    fn read(&self, image: &image::GrayImage) -> passport_scanner_core::Result<String> {
        let text = self.0.read(image)?;
        if std::env::var_os("MRZ_REVIEW_SHAPES").is_some() {
            for line in text.lines() {
                let shape: String = line
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphabetic() {
                            'A'
                        } else if c.is_ascii_digit() {
                            '0'
                        } else {
                            c
                        }
                    })
                    .collect();
                println!("shape {}: {}", line.len(), shape);
            }
            println!("end_attempt");
        }
        Ok(text)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let reader = MrzReader::new(Shapes(Tesseract::default()));
    for path in std::env::args().skip(1) {
        let frame = Frame {
            width: 0,
            height: 0,
            format: PixelFormat::Jpeg,
            source: LightSource::White,
            data: std::fs::read(path)?,
        };
        let (reading, angle) = reader.read_frame_oriented(&frame)?;
        let lengths: Vec<usize> = reading.text.lines().map(str::len).collect();
        let parsed = reading.lines.as_ref().and_then(|lines| {
            passport_scanner_mrz::parse(&lines.iter().map(String::as_str).collect::<Vec<_>>()).ok()
        });
        println!(
            "rotation={angle}, ocr_line_lengths={lengths:?}, complete_zone={}, checks_valid={}",
            parsed.is_some(),
            parsed.as_ref().is_some_and(|d| d.checksums_valid())
        );
    }
    Ok(())
}
