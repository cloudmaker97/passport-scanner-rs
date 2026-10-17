//! Offline SHD comparison. Does not open a scanner or modify calibration files.
use passport_scanner_core::LightSource;
use passport_scanner_imaging::{Enhance, Shading, flatfield};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() < 4 {
        return Err(
            "usage: reprocess <raw.jpg> <calibration.shd> <output.jpg> [--legacy-rows] [--infrared]".into()
        );
    }
    let mut bytes = std::fs::read(&args[2])?;
    if args.iter().any(|s| s == "--legacy-rows") {
        // Undo the corrected loader's row reversal to reproduce the old bug.
        let row = flatfield::GRID_W * 3 * 4;
        let start = 0;
        if bytes.len() != row * flatfield::GRID_H + flatfield::TRAILER * 4 {
            return Err("unexpected SHD length".into());
        }
        let original = bytes.clone();
        for y in 0..flatfield::GRID_H {
            let from = start + (flatfield::GRID_H - 1 - y) * row;
            let to = start + y * row;
            bytes[to..to + row].copy_from_slice(&original[from..from + row]);
        }
    }
    let shading = Shading::parse(&bytes)?;
    let mut image = image::ImageReader::open(&args[1])?.decode()?.to_rgb8();
    let source = if args.iter().any(|s| s == "--infrared") {
        LightSource::Infrared
    } else {
        LightSource::White
    };
    Enhance::for_documents_with(Some(shading), source).run(&mut image);
    let file = std::fs::File::create(&args[3])?;
    image::codecs::jpeg::JpegEncoder::new_with_quality(
        file,
        passport_scanner_imaging::JPEG_QUALITY,
    )
    .encode_image(&image)?;
    println!("{}: {}x{}", args[3], image.width(), image.height());
    Ok(())
}
