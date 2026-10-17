//! Integration tests use independent vertical-shear geometry, never the
//! production deskew rotator to generate the input being tested.
use super::*;
use image::{GrayImage, Luma};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

const SPECIMEN: &str = "I<UTOD231458907<<<<<<<<<<<<<<<\n7408122F1204159UTO<<<<<<<<<<<6\nERIKSSON<<ANNA<MARIA<<<<<<<<<<";

fn card(slope_degrees: f32) -> GrayImage {
    // Vertically symmetric synthetic glyphs make the independent centroid
    // regression unbiased by which glyph appears at the left or right edge.
    let glyphs = [
        [31u8, 4, 4, 4, 4, 4, 31],
        [17, 17, 17, 31, 17, 17, 17],
        [14, 17, 17, 17, 17, 17, 14],
        [2, 4, 8, 16, 8, 4, 2],
    ];
    let mut upright = GrayImage::from_pixel(760, 480, Luma([235]));
    for row in 0..3 {
        for character in 0..44 {
            let glyph = &glyphs[character % glyphs.len()];
            for (gy, bits) in glyph.iter().enumerate() {
                for gx in 0..5 {
                    if bits & (1 << (4 - gx)) != 0 {
                        for dy in 0..2 {
                            for dx in 0..2 {
                                upright.put_pixel(
                                    (24 + character * 16 + gx * 2 + dx) as u32,
                                    (335 + row * 39 + gy * 2 + dy) as u32,
                                    Luma([25]),
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    let slope = slope_degrees.to_radians().tan();
    GrayImage::from_fn(760, 480, |x, y| {
        let source_y = y as f32 - slope * (x as f32 - 379.5);
        let top = source_y.floor() as i32;
        let fraction = source_y - top as f32;
        let sample = |y: i32| {
            if (0..480).contains(&y) { upright.get_pixel(x, y as u32).0[0] as f32 } else { 235. }
        };
        Luma([(sample(top) * (1. - fraction) + sample(top + 1) * fraction).round() as u8])
    })
}

fn frame(pixels: GrayImage) -> Frame {
    Frame {
        width: pixels.width(),
        height: pixels.height(),
        format: PixelFormat::Gray8,
        source: LightSource::White,
        data: pixels.into_raw(),
    }
}

#[derive(Debug)]
struct HorizontalOnly {
    calls: Arc<AtomicUsize>,
    bad_checksum: bool,
}

impl Engine for HorizontalOnly {
    fn read(&self, image: &GrayImage) -> Result<String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if image.width() < image.height() * 2 {
            return Ok(String::new());
        }
        let mut rows = vec![0usize; image.height() as usize];
        for (_, y, pixel) in image.enumerate_pixels() {
            if pixel.0[0] < 100 {
                rows[y as usize] += 1;
            }
        }
        let threshold = rows.iter().copied().max().unwrap_or(0) / 12;
        let mut bands = Vec::new();
        let mut start = None;
        for (y, count) in rows.iter().enumerate() {
            if *count > threshold {
                start.get_or_insert(y);
            } else if let Some(top) = start.take() {
                bands.push((top, y));
            }
        }
        if let Some(top) = start {
            bands.push((top, rows.len()));
        }
        if bands.len() != 3 {
            return Ok(String::new());
        }
        // Regress each text line separately: a rotation also shifts entire
        // lines horizontally, which biases a single regression over all rows.
        for (top, bottom) in bands {
            let (mut n, mut sx, mut sy, mut sxx, mut sxy) = (0f64, 0f64, 0f64, 0f64, 0f64);
            for y in top..bottom {
                for x in 0..image.width() {
                    if image.get_pixel(x, y as u32).0[0] < 100 {
                        let (x, y) = (f64::from(x), y as f64);
                        n += 1.;
                        sx += x;
                        sy += y;
                        sxx += x * x;
                        sxy += x * y;
                    }
                }
            }
            let denominator = n * sxx - sx * sx;
            if n < 200.
                || denominator <= 0.
                || ((n * sxy - sx * sy) / denominator).abs() > 0.2f64.to_radians().tan()
            {
                return Ok(String::new());
            }
        }
        Ok(if self.bad_checksum {
            SPECIMEN.replace("D231458907", "D231458908")
        } else {
            SPECIMEN.into()
        })
    }
}

fn reader(bad_checksum: bool) -> (MrzReader<HorizontalOnly>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (MrzReader::new(HorizontalOnly { calls: calls.clone(), bad_checksum }), calls)
}

#[test]
fn independent_small_shear_recovers_through_all_cardinal_orientations() {
    for slope in [-1., 1.] {
        let pixels = card(slope);
        for (pixels, expected_rotation) in [
            (pixels.clone(), 0),
            (image::imageops::rotate180(&pixels), 180),
            (image::imageops::rotate90(&pixels), -90),
            (image::imageops::rotate270(&pixels), 90),
        ] {
            let frame = frame(pixels);
            let original = frame.data.clone();
            let (reader, calls) = reader(false);
            let (reading, rotation) = reader.read_frame_oriented(&frame).unwrap();
            assert_eq!(rotation, expected_rotation, "slope={slope}");
            assert!(
                verified(&reading),
                "slope={slope} rotation={rotation} calls={}",
                calls.load(Ordering::Relaxed)
            );
            assert!(calls.load(Ordering::Relaxed) >= 2, "sloped original must need a retry");
            assert!(reading.strip.w > 0 && reading.strip.h > 0);
            assert!(reading.strip.x + reading.strip.w <= frame.width);
            assert!(reading.strip.y + reading.strip.h <= frame.height);
            assert_eq!(frame.data, original);
        }
    }
}

#[test]
fn straight_verified_input_keeps_one_call_fast_path() {
    let (reader, calls) = reader(false);
    let frame = frame(card(0.));
    let original = frame.data.clone();
    let result = reader.recognize_oriented(std::slice::from_ref(&frame)).unwrap();
    assert!(result.document.checksums_valid());
    assert!(!result.document.fields.is_empty());
    assert_eq!(result.rotation, Some(0));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(frame.data, original);
}

#[test]
fn deskew_never_promotes_a_bad_checksum_to_document_fields() {
    let (reader, calls) = reader(true);
    let frame = frame(card(1.));
    let original = frame.data.clone();
    let result = reader.recognize_oriented(std::slice::from_ref(&frame)).unwrap();
    assert!(result.document.fields.is_empty());
    assert_eq!(result.rotation, None);
    assert!(calls.load(Ordering::Relaxed) >= 2);
    assert_eq!(frame.data, original);
}
