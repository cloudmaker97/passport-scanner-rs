//! Report what the imaging stages measure on a file, without changing it.
//!
//! ```text
//! cargo run -p passport-scanner-app --bin passport-scanner-probe -- scans/white-raw.jpg
//! ```

use passport_scanner_imaging::{AutoLevels, Sharpen, Stage, WhiteBalance, balance, levels, luma};

/// Report the colour of the brightest part of an image: the card's paper.
fn paper(image: &image::RgbImage) -> [u8; 3] {
    let mut luminance = [0u64; 256];
    for p in image.pixels() {
        luminance[luma(p.0) as usize] += 1;
    }
    let floor = levels::platen(&luminance).unwrap_or(0);

    let mut channels = [[0u64; 256]; 3];
    for p in image.pixels() {
        if luma(p.0) > floor {
            for (h, v) in channels.iter_mut().zip(p.0) {
                h[v as usize] += 1;
            }
        }
    }
    channels.map(|h| balance::highlight(&h, balance::HIGHLIGHT).unwrap_or(0))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let image = image::ImageReader::open(&path)?.decode()?.to_rgb8();

        let mut luminance = [0u64; 256];
        for p in image.pixels() {
            luminance[luma(p.0) as usize] += 1;
        }
        let floor = levels::platen(&luminance);
        println!("{path}  {}x{}", image.width(), image.height());
        println!("  platen split: {floor:?}");

        let floor = floor.unwrap_or(0);
        let mut channels = [[0u64; 256]; 3];
        let mut lit = 0u64;
        for p in image.pixels() {
            if luma(p.0) <= floor {
                continue;
            }
            lit += 1;
            for (h, v) in channels.iter_mut().zip(p.0) {
                h[v as usize] += 1;
            }
        }
        println!("  lit pixels: {lit} of {}", u64::from(image.width()) * u64::from(image.height()));

        let whites = channels.map(|h| balance::highlight(&h, balance::HIGHLIGHT).unwrap_or(0));
        let means = channels.map(|h| {
            let n: u64 = h.iter().sum();
            let s: u64 = h.iter().enumerate().map(|(v, c)| v as u64 * c).sum();
            if n == 0 { 0.0 } else { s as f64 / n as f64 }
        });
        println!("  white points  r {} g {} b {}", whites[0], whites[1], whites[2]);
        println!("  lit means     r {:.1} g {:.1} b {:.1}", means[0], means[1], means[2]);
        if let Some(g) = balance::gains(whites.map(f32::from)) {
            println!("  gains         r {:.3} g {:.3} b {:.3}", g[0], g[1], g[2]);
        }

        let lit_histogram = levels::above(&luminance, floor);
        println!(
            "  levels on lit: {:?}",
            levels::black_white(&lit_histogram, levels::Cuts::default())
        );

        // Walk the stages, reporting the paper colour after each. This is what
        // says which stage is responsible for a cast, rather than guessing.
        let stages: [Box<dyn Stage>; 3] = [
            Box::new(WhiteBalance::for_documents()),
            Box::new(AutoLevels::for_documents()),
            Box::new(Sharpen::default()),
        ];
        let mut walked = image.clone();
        println!("  paper {:?} before any stage", paper(&walked));
        for stage in &stages {
            stage.apply(&mut walked);
            println!("  paper {:?} after {}", paper(&walked), stage.name());
        }
    }
    Ok(())
}
