//! Measure how this unit lights its platen, and save the result.
//!
//! The lamp does not light the platen evenly - on the reference unit by a
//! factor of five across the frame - and no tone curve can undo that, because
//! the two ends of the frame need different ones. What fixes it is a per-pixel
//! map of the unevenness, measured once per unit and applied to every capture
//! afterwards. This measures it.
//!
//! Put a sheet of plain white paper over the whole platen, close it, and run:
//!
//! ```text
//! passport-scanner-calibrate
//! ```
//!
//! The paper is the reference: whatever it returns is what "white" looks like
//! at each point, and the correction is what turns that into one number
//! everywhere. Use matt paper, unprinted on both sides, and make sure it
//! covers the platen corner to corner - an uncovered patch calibrates as
//! though the lamp were dark there.
//!
//! The vendor's own calibration is never written over. What is produced here
//! is read first and the vendor's is the fallback, so removing one file goes
//! back to it.

#[cfg(not(windows))]
fn main() {
    eprintln!("This build has no transport for the current platform.");
}

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use passport_scanner_core::LightSource;
    use passport_scanner_imaging::flatfield;

    let keep = std::env::args().any(|a| a == "--keep-capture");

    let (mut scanner, model) = passport_scanner_app::open()?;
    let product = model.product_id();
    println!("{} - measuring the platen under the visible lamp\n", model.name());

    // The reference has to be the sensor's own view, so nothing is corrected
    // or cropped on the way here.
    let frames = scanner.scan(&[LightSource::White])?;
    let frame = frames.first().ok_or("the scanner returned no frame")?;
    let image = image::load_from_memory(&frame.data)?.to_rgb8();
    println!("captured {}x{}", image.width(), image.height());

    let reference = passport_scanner_app::calibration::Reference::of(&image);
    println!(
        "{:.0}% of the frame is dark, {:.0}% is well lit",
        reference.dark * 100.0,
        reference.lit * 100.0
    );

    let shading = passport_scanner_app::calibration::measure(
        &image,
        passport_scanner_device::protocol::VENDOR_ID,
        product,
    )?;
    let path = passport_scanner_app::calibration::own_file(
        passport_scanner_device::protocol::VENDOR_ID,
        product,
    );
    println!("\nwritten to {}", path.display());

    if keep {
        let beside = path.with_extension("jpg");
        std::fs::write(&beside, &frame.data)?;
        println!("reference capture kept at {}", beside.display());
    }

    // Say what it measured, because a calibration that implies an implausible
    // range is one worth looking at rather than trusting.
    let mut lowest = f32::MAX;
    let mut highest = 0f32;
    for y in (0..flatfield::GRID_H).step_by(7) {
        for x in (0..flatfield::GRID_W).step_by(7) {
            if let Some(g) = shading.at(x, y, 1) {
                let reference = flatfield::TARGET_VISIBLE / g;
                lowest = lowest.min(reference);
                highest = highest.max(reference);
            }
        }
    }
    println!(
        "the platen reads {lowest:.0} to {highest:.0} across the frame, a factor of {:.1}",
        highest / lowest.max(1.0)
    );
    println!("\nScans from now on use this. Delete the file to go back to the vendor's.");
    Ok(())
}
