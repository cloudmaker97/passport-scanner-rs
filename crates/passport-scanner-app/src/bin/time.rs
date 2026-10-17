//! Find out where the time in a scan goes, and what cutting it costs.
//!
//! Each run changes one thing, so the parts can be told apart by difference
//! rather than by guesswork. Two things are reported alongside the time,
//! because a saving that costs an image is not a saving:
//!
//! - the tonal spread of the white capture, which is what there is to read;
//! - how far the white and infrared frames differ from each other. If too few
//!   frames are thrown away after a lamp change, the stream hands back one
//!   exposed under the previous lamp and the two come out nearly identical.
//!   That is the failure this is here to catch.
//!
//! ```text
//! cargo run --release -p passport-scanner-app --bin passport-scanner-time
//! ```

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(windows))]
    {
        eprintln!("This build has no transport for the current platform.");
        Ok(())
    }

    #[cfg(windows)]
    {
        use image::RgbImage;
        use passport_scanner_core::{Frame, LightSource};
        use passport_scanner_imaging::{levels, luma};
        use std::time::{Duration, Instant};

        fn decode(frame: &Frame) -> Option<RgbImage> {
            image::load_from_memory(&frame.data).ok().map(|i| i.to_rgb8())
        }

        /// How far apart the lit tones sit - what there is to read.
        fn spread(image: &RgbImage) -> u8 {
            let mut histogram = [0u64; 256];
            for p in image.pixels() {
                histogram[luma(p.0) as usize] += 1;
            }
            let floor = levels::platen(&histogram).unwrap_or(0);
            match levels::black_white(&levels::above(&histogram, floor), levels::Cuts::default()) {
                Some(p) => p.white.saturating_sub(p.black),
                None => 0,
            }
        }

        /// Mean absolute difference between two frames, in levels.
        fn difference(a: &RgbImage, b: &RgbImage) -> f64 {
            if a.dimensions() != b.dimensions() {
                return f64::NAN;
            }
            let total: u64 =
                a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
            total as f64 / a.as_raw().len() as f64
        }

        let sources = [LightSource::White, LightSource::Infrared];
        println!("{:<26} {:>9} {:>8} {:>12}", "run", "capture", "spread", "white vs ir");

        // Repeated, because one run of each is not enough to tell a setting
        // that works from one that happened to work.
        let runs: [(&str, u64, usize); 8] = [
            ("100 ms settle", 100, 2),
            ("50 ms settle", 50, 2),
            ("0 ms settle", 0, 2),
            ("0 ms, 1 start flush", 0, 1),
            ("100 ms settle", 100, 2),
            ("50 ms settle", 50, 2),
            ("0 ms settle", 0, 2),
            ("0 ms, 1 start flush", 0, 1),
        ];

        for (label, settle, flush) in runs {
            let (mut scanner, _) = passport_scanner_app::open()?;
            scanner.set_settle(Duration::from_millis(settle));
            scanner.set_flush(flush);

            let started = Instant::now();
            let frames = scanner.scan(&sources)?;
            let cost = started.elapsed();

            let white = frames.iter().find(|f| f.source == LightSource::White).and_then(decode);
            let infrared =
                frames.iter().find(|f| f.source == LightSource::Infrared).and_then(decode);
            match (white, infrared) {
                (Some(w), Some(i)) => println!(
                    "{label:<26} {cost:>9.2?} {:>8} {:>12.1}",
                    spread(&w),
                    difference(&w, &i)
                ),
                _ => println!("{label:<26} {cost:>9.2?}   (a frame would not decode)"),
            }
            drop(scanner);
        }
        Ok(())
    }
}
