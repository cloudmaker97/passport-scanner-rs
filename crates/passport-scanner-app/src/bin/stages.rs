//! Write out what each imaging stage does to a capture, one file per stage.
//!
//! When a result looks wrong there are two questions — which stage did it, and
//! would it look better without that stage — and a pipeline that only emits its
//! final answer makes both unanswerable. This runs the stages one at a time
//! over a raw capture and saves the frame after each, so the two can be looked
//! at side by side.
//!
//! ```text
//! cargo run --release -p passport-scanner-app --bin passport-scanner-stages -- scans/white-raw.jpg
//! ```

use passport_scanner_core::{Frame, LightSource, PixelFormat};
use passport_scanner_imaging::Enhance;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: passport-scanner-stages <image> [vendor] [product]");
        std::process::exit(2);
    };
    let vendor = parse_arg(2, 0x07B3);
    let product = parse_arg(3, 0x1A05);

    let source = if path.contains("infrared") || path.contains("-ir") {
        LightSource::Infrared
    } else {
        LightSource::White
    };

    let image = image::ImageReader::open(&path)?.decode()?.to_rgb8();
    println!("{path}  {}x{}  under {source}", image.width(), image.height());

    let shading = passport_scanner_app::calibration::for_documents(vendor, product)?;
    match &shading {
        Some(_) => println!("calibration: found"),
        None => println!("calibration: none on this machine"),
    }

    let pipeline = Enhance::for_documents_with(shading, source);

    let stem = std::path::Path::new(&path).with_extension("");
    let stem = stem.to_string_lossy().into_owned();

    let mut running = image;
    write(&format!("{stem}-0-asis.jpg"), &running)?;
    println!("\n{:<14} {:>11} {:>8} {:>22}", "after", "size", "spread", "mean r,g,b");
    report("(nothing)", &running);

    let mut writes = Vec::new();
    let mut n = 0;
    pipeline.run_observed(&mut running, |name, image| {
        n += 1;
        let filename = name.replace(' ', "-");
        writes.push(write(&format!("{stem}-{n}-{filename}.jpg"), image));
        report(name, image);
    });
    for result in writes {
        result?;
    }
    write(&format!("{stem}-9-enhance.jpg"), &running)?;
    report("enhance", &running);
    println!("\nwritten beside {path}");
    Ok(())
}

fn parse_arg(n: usize, fallback: u16) -> u16 {
    std::env::args()
        .nth(n)
        .and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(fallback)
}

/// Tones and colour of the lit part, which is what "looks wrong" usually means.
fn report(label: &str, image: &image::RgbImage) {
    use passport_scanner_imaging::{levels, luma};

    let mut histogram = [0u64; 256];
    for p in image.pixels() {
        histogram[luma(p.0) as usize] += 1;
    }
    let floor = levels::platen(&histogram).unwrap_or(0);

    let mut sums = [0u64; 3];
    let mut lit = 0u64;
    for p in image.pixels() {
        if luma(p.0) > floor {
            lit += 1;
            for (sum, value) in sums.iter_mut().zip(p.0) {
                *sum += u64::from(value);
            }
        }
    }
    let spread = levels::black_white(&levels::above(&histogram, floor), levels::Cuts::default())
        .map_or(0, |p| p.white.saturating_sub(p.black));
    let mean = sums.map(|s| if lit == 0 { 0.0 } else { s as f64 / lit as f64 });

    // And the ink on its own. A cast that shows in the print but not in the
    // paper is a different fault from one that shows in both: the paper is
    // what a white balance corrects against, so if that is neutral and the
    // ink is not, the correction is multiplicative where it should not be.
    let mut dark = [0u64; 3];
    let mut n = 0u64;
    for p in image.pixels() {
        if luma(p.0) < 128 {
            n += 1;
            for (sum, value) in dark.iter_mut().zip(p.0) {
                *sum += u64::from(value);
            }
        }
    }
    let ink = dark.map(|s| if n == 0 { 0.0 } else { s as f64 / n as f64 });

    println!(
        "{label:<14} {:>5}x{:<5} {spread:>8} {:>7.1}{:>7.1}{:>7.1} {:>7.1}{:>7.1}{:>7.1}",
        image.width(),
        image.height(),
        mean[0],
        mean[1],
        mean[2],
        ink[0],
        ink[1],
        ink[2]
    );
}

fn write(path: &str, image: &image::RgbImage) -> Result<(), Box<dyn std::error::Error>> {
    let mut frame = Frame {
        width: image.width(),
        height: image.height(),
        format: PixelFormat::Rgb8,
        source: LightSource::White,
        data: image.as_raw().clone(),
    };
    // Through the same encoder the pipeline uses, so the files are comparable.
    passport_scanner_app::enhance_none(std::slice::from_mut(&mut frame))?;
    std::fs::write(path, &frame.data)?;
    Ok(())
}
