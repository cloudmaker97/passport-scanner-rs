//! Sweep one of the capture stack's image controls through the real scan path.
//!
//! The stack runs its own exposure loop, so these controls - not the sensor's
//! exposure register - settle how a frame comes out. Sweeping them inside the
//! actual scan sequence matters: doing it alongside gives a different answer,
//! because the sequence also loads the unit's register tables.
//!
//! ```text
//! cargo run -p passport-scanner-app --bin passport-scanner-expose                 # Brightness 0..40
//! cargo run -p passport-scanner-app --bin passport-scanner-expose Contrast 1 32
//! cargo run -p passport-scanner-app --bin passport-scanner-expose Gamma 1 12
//! ```

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(windows))]
    {
        eprintln!("This build has no transport for the current platform.");
        Ok(())
    }

    #[cfg(windows)]
    {
        use passport_scanner_core::LightSource;

        let control = std::env::args().nth(1).unwrap_or_else(|| "Brightness".to_owned());
        let lo: i64 = std::env::args().nth(2).and_then(|v| v.parse().ok()).unwrap_or(0);
        let hi: i64 = std::env::args().nth(3).and_then(|v| v.parse().ok()).unwrap_or(40);

        let (mut scanner, model) = passport_scanner_app::open()?;
        println!("{} - sweeping {control} from {lo} to {hi}\n", model.name());
        std::fs::create_dir_all("scans/sweep")?;

        println!(
            "{:>8}   {:>5}   {:>6}   {:>8}   {:>8}",
            "value", "mean", "spread", "near max", "bytes"
        );

        let mut best: Option<(i64, f32, Vec<u8>)> = None;
        let steps = 8i64;
        for i in 0..=steps {
            let value = lo + (hi - lo) * i / steps;
            scanner.set_white_controls(vec![(control.clone(), value)]);
            let frame = scanner.capture_once(LightSource::White)?;
            let s = lit_stats(&frame.data)?;

            println!(
                "{value:>8}   {:>5.0}   {:>6.1}   {:>7.1}%   {:>8}",
                s.mean,
                s.spread,
                s.near_max * 100.0,
                frame.data.len()
            );
            std::fs::write(format!("scans/sweep/{control}-{value:03}.jpg"), &frame.data)?;

            // Legibility is contrast, not brightness: take the setting that
            // spreads the lit tones furthest without pushing them to the ceiling.
            if s.near_max <= 0.01 && best.as_ref().is_none_or(|(_, b, _)| s.spread > *b) {
                best = Some((value, s.spread, frame.data.clone()));
            }
        }

        match best {
            Some((value, spread, data)) => {
                std::fs::write("scans/best-white.jpg", &data)?;
                println!("\nbest: {control} {value}, tonal spread {spread:.1}");
                println!("written to scans/best-white.jpg");
            }
            None => println!("\nEvery setting pushed the highlights to the ceiling."),
        }
        Ok(())
    }
}

#[cfg(windows)]
struct Lit {
    /// Mean of the lit pixels.
    mean: f32,
    /// Standard deviation of the lit pixels - how much tone there is to read.
    spread: f32,
    /// Fraction of lit pixels at the ceiling.
    near_max: f32,
}

/// Statistics over the lit part of the frame.
///
/// Locating the document by its bounding box is unreliable here: the platen
/// throws bright reflections that drag the box out to the edges, and the mean
/// then describes the background rather than the card. Measuring over pixels
/// that are lit at all avoids having to find the card first.
#[cfg(windows)]
fn lit_stats(jpeg: &[u8]) -> Result<Lit, Box<dyn std::error::Error>> {
    let grey = image::load_from_memory(jpeg)?.to_luma8();
    let lit: Vec<u8> = grey.as_raw().iter().copied().filter(|v| *v >= 128).collect();
    if lit.is_empty() {
        return Ok(Lit { mean: 0.0, spread: 0.0, near_max: 0.0 });
    }

    let n = lit.len() as f32;
    let mean = lit.iter().map(|v| f32::from(*v)).sum::<f32>() / n;
    let variance = lit.iter().map(|v| (f32::from(*v) - mean).powi(2)).sum::<f32>() / n;
    let near_max = lit.iter().filter(|v| **v >= 245).count() as f32 / n;

    Ok(Lit { mean, spread: variance.sqrt(), near_max })
}
