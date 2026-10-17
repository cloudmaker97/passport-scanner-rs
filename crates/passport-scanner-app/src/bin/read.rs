//! Read a document out of an image file, without a scanner.
//!
//! Useful for checking the recognition path on its own, and for re-reading a
//! capture that was saved earlier.
//!
//! ```text
//! cargo run -p passport-scanner-app --bin passport-scanner-read -- scans/infrared.jpg
//! ```

use passport_scanner_core::{DocumentRecognizer, Frame, LightSource, PixelFormat};
use passport_scanner_recognize::{MrzReader, Tesseract};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: passport-scanner-read <image>");
        std::process::exit(2);
    };

    let data = std::fs::read(&path)?;
    let decoded = image::ImageReader::open(&path)?.with_guessed_format()?.decode()?;
    let frame = Frame {
        width: decoded.width(),
        height: decoded.height(),
        format: PixelFormat::Jpeg,
        source: LightSource::Infrared,
        data,
    };

    let engine = Tesseract::default();
    println!("engine: {}", engine.binary.display());

    let reader = MrzReader::new(engine);

    // Show what the engine saw before judging what came of it: a zone that is
    // plainly legible but does not parse is almost always lost in the repair,
    // not in the OCR, and that is invisible from the outcome alone.
    match reader.read_frame_raw(&frame) {
        Ok(reading) => {
            println!(
                "
zone strip: {}x{} at {},{}",
                reading.strip.w, reading.strip.h, reading.strip.x, reading.strip.y
            );
            println!(
                "
engine said:"
            );
            for line in reading.text.lines().filter(|l| !l.trim().is_empty()) {
                println!("  {:>2}  {line}", line.chars().count());
            }
            match reading.lines {
                Some(lines) => {
                    println!(
                        "
accepted as a zone:"
                    );
                    for l in &lines {
                        println!("  {l}");
                    }
                }
                None => println!(
                    "
None of that is a complete zone."
                ),
            }
        }
        Err(e) => println!("could not read the frame: {e}"),
    }

    match reader.read_frame(&frame) {
        Ok(lines) => {
            println!("\nzone as read:");
            for l in &lines {
                println!("  {l}");
            }
            let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
            match passport_scanner_mrz::parse(&borrowed) {
                Ok(doc) => {
                    println!("\nfields:");
                    for f in doc.fields.values() {
                        let mark = match f.checksum_ok {
                            Some(true) => "check digit ok",
                            Some(false) => "CHECK DIGIT FAILED",
                            None => "",
                        };
                        println!("  {:>16?}  {:<30} {mark}", f.kind, f.value);
                    }
                    println!(
                        "\ncheck digits: {}",
                        if doc.checksums_valid() { "all match" } else { "one or more failed" }
                    );
                }
                Err(e) => println!("\nthe zone did not parse: {e}"),
            }
        }
        Err(e) => {
            println!("\nno zone read: {e}");
            // Show what the engine saw, so a failure is diagnosable.
            if let Ok(doc) = reader.recognize(std::slice::from_ref(&frame)) {
                println!("recogniser returned {} field(s)", doc.fields.len());
            }
        }
    }
    Ok(())
}
