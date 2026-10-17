//! Correcting for a platen that is not lit evenly.
//!
//! The lamp is a strip of LEDs behind a diffuser a few millimetres from the
//! glass, so what it puts on the platen is nothing like flat. The unit's own
//! calibration says how far from flat: on the reference scanner the white
//! reference it implies runs from 48 to 240 across the frame, a factor of five
//! between the brightest part of the platen and the dimmest. A card laid
//! across that boundary is half well exposed and half not, and no single tone
//! curve can rescue both halves - which is exactly what made the output here
//! swing between legible and useless from one scan to the next.
//!
//! The shipped driver does this first, before anything else touches the image,
//! and so does this. Everything downstream - the white balance, the levels,
//! finding the document - assumes an evenly lit frame, and only this makes
//! that true.
//!
//! # The file
//!
//! `Calibration_*.shd`, written by the vendor's calibration program into
//! `C:\Users\Public\Documents\Plustek\VID_xxxx&PID_xxxx\`. Headerless
//! little-endian `float32`:
//!
//! ```c
//! struct ShadingFile {
//!     float32 gain[486][648][3];     // bottom-up rows, interleaved BGR
//!     float32 geometry[18];         // crop and resize metadata
//! };
//! ```
//!
//! 648×486 is the five-megapixel sensor downsampled by four. The arithmetic
//! that settles it: 3 779 208 / 4 = 944 802, and 944 802 − 18 = 648 × 486 × 3.
//!
//! Each coefficient is `TARGET / white_reference`, where the reference is the
//! eight-bit value that pixel returned looking at a blank white platen. That
//! this is the formula and not a guess: across the shipped files, `240 / gain`
//! comes out a whole number in 100.00 % of coefficients sampled.

use passport_scanner_core::{Error, Result};

/// Width of the calibration grid.
pub const GRID_W: usize = 648;
/// Height of the calibration grid.
pub const GRID_H: usize = 486;
/// Geometry floats after the coefficient grid, not a prologue.
///
/// The observed trailers contain crop rectangles and resize scales. Treating
/// them as gains shifted the grid six pixels sideways and interpreted crop
/// coordinates such as 2517 as enormous correction factors. The metadata is
/// preserved when loading and saving; this pipeline finds its own crop.
pub const TRAILER: usize = 18;

/// What a corrected white is scaled to.
///
/// The shipped code picks this by searching the file's own name: 215 where it
/// contains "IR", 240 otherwise. Left under the target's own ceiling, so the
/// correction does not spend its headroom pushing the platen to pure white.
pub const TARGET_VISIBLE: f32 = 240.0;

/// The same, for an infrared capture.
pub const TARGET_INFRARED: f32 = 215.0;

/// Low percentile for diagnostic measurements of scene shadows.
/// This is not a measured sensor black level.
pub const BLACK_POINT: f32 = 0.01;

/// Largest correction to actually apply.
///
/// Large gains amplify noise in poorly illuminated regions. Values above
/// this limit are left uncalibrated. Actual gains in the observed vendor
/// files stay below this; the old apparent values above 2500 were geometry
/// metadata incorrectly read as gain coefficients.
pub const MAX_GAIN: f32 = 8.0;

/// A unit's flat-field calibration.
#[derive(Debug, Clone)]
pub struct Shading {
    /// Per-pixel gain on the calibration grid, interleaved, in RGB order.
    ///
    /// The file stores bottom-up rows and BGR channels, the Windows bitmap
    /// convention. Memory uses top-down rows and RGB, matching decoded images.
    gain: Vec<f32>,
    /// Vendor geometry metadata, retained unchanged on a file roundtrip.
    trailer: [f32; TRAILER],
}

impl Shading {
    /// Read a `.shd` file.
    ///
    /// # Errors
    /// [`Error::Malformed`] if the file is not the size the format requires.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let floats = bytes.len() / 4;
        let wanted = TRAILER + GRID_W * GRID_H * 3;
        if bytes.len() % 4 != 0 || floats != wanted {
            return Err(Error::Malformed(format!(
                "a shading file holds {wanted} floats, this one holds {floats}"
            )));
        }

        let mut gain = vec![0f32; GRID_W * GRID_H * 3];
        for (i, slot) in gain.iter_mut().enumerate() {
            // SHD follows DIB storage in both axes: BGR channels and bottom-up
            // rows. The matching vendor reference JPEG confirms the row origin.
            let channel = i % 3;
            let x = (i / 3) % GRID_W;
            let y = i / (GRID_W * 3);
            let source = ((GRID_H - 1 - y) * GRID_W + x) * 3 + (2 - channel);
            let at = source * 4;
            *slot = f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        }
        let trailer = std::array::from_fn(|i| {
            let at = (GRID_W * GRID_H * 3 + i) * 4;
            f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
        });
        Ok(Self { gain, trailer })
    }

    /// The gain at a point on the grid, or `None` where there is no usable
    /// reading.
    ///
    /// Out of range, zero, or past [`MAX_GAIN`] all mean the same thing to a
    /// caller: this pixel was never calibrated.
    #[must_use]
    pub fn at(&self, x: usize, y: usize, channel: usize) -> Option<f32> {
        if x >= GRID_W || y >= GRID_H || channel >= 3 {
            return None;
        }
        let g = self.gain[(y * GRID_W + x) * 3 + channel];
        (g.is_finite() && g > 0.0 && g <= MAX_GAIN).then_some(g)
    }

    /// The gain for a point in an image of `width` by `height`, interpolated.
    ///
    /// The grid is a quarter of the sensor in each direction, so every image
    /// pixel falls between four coefficients. Taking the nearest one would
    /// print the grid onto the image as four-pixel blocks; the lighting it
    /// describes is smooth, so the reading should be too.
    ///
    /// Returns `None` if any of the four corners is uncalibrated, so that the
    /// uncovered edges of the frame come out black rather than smeared.
    #[must_use]
    pub fn sample(&self, x: u32, y: u32, width: u32, height: u32, channel: usize) -> Option<f32> {
        if width == 0 || height == 0 {
            return None;
        }
        let fx = scale(x, width, GRID_W);
        let fy = scale(y, height, GRID_H);
        let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(GRID_W - 1), (y0 + 1).min(GRID_H - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);

        let g00 = self.at(x0, y0, channel)?;
        let g10 = self.at(x1, y0, channel)?;
        let g01 = self.at(x0, y1, channel)?;
        let g11 = self.at(x1, y1, channel)?;

        let top = g00 + (g10 - g00) * tx;
        let bottom = g01 + (g11 - g01) * tx;
        Some(top + (bottom - top) * ty)
    }
}

/// A low percentile of each channel, for scene diagnostics.
///
/// A scene percentile is not a dark-frame measurement: a sheet filling the
/// platen returns the sheet's own tone here. Do not subtract this from pixels
/// before applying SHD gains, which encode `target / white_reference` without
/// a dark offset. Offset correction requires a separately measured dark frame
/// and the matching denominator `white_reference - dark_reference`.
#[must_use]
pub fn black_point(histograms: &[[u64; 256]; 3], fraction: f32) -> [u8; 3] {
    let mut out = [0u8; 3];
    for (slot, histogram) in out.iter_mut().zip(histograms) {
        let total: u64 = histogram.iter().sum();
        if total == 0 {
            continue;
        }
        let want = (total as f32 * fraction) as u64;
        let mut seen = 0u64;
        for (value, count) in histogram.iter().enumerate() {
            seen += count;
            if seen > want {
                *slot = value as u8;
                break;
            }
        }
    }
    out
}

impl Shading {
    /// Build a calibration from a capture of a blank white reference.
    ///
    /// The inverse of what [`Shading::sample`] does with one: the reference is
    /// whatever the sensor returned looking at a surface that should read as
    /// `target` everywhere, so the gain that makes it do so is
    /// `target / reference`.
    ///
    /// The capture is averaged down to the calibration grid rather than
    /// sampled at it. A single pixel carries the sensor's noise and a dust
    /// speck carries someone else's; sixteen averaged carry neither, and what
    /// is being measured - how the lamp lights the platen - does not vary over
    /// sixteen pixels anyway.
    ///
    /// A point that reads as good as nothing is left uncalibrated rather than
    /// given an enormous gain. Those are the corners outside the lens.
    #[must_use]
    pub fn from_reference(image: &image::RgbImage, target: f32) -> Self {
        let (w, h) = (image.width(), image.height());
        let trailer = whole_frame_geometry(w, h);
        let mut gain = vec![0f32; GRID_W * GRID_H * 3];
        if w == 0 || h == 0 || !target.is_finite() || target <= 0.0 {
            return Self { gain, trailer };
        }

        for gy in 0..GRID_H {
            for gx in 0..GRID_W {
                // The block of the capture this grid point stands for.
                let x0 = (gx as u32 * w / GRID_W as u32).min(w.saturating_sub(1));
                let x1 = (((gx + 1) as u32 * w) / GRID_W as u32).clamp(x0 + 1, w);
                let y0 = (gy as u32 * h / GRID_H as u32).min(h.saturating_sub(1));
                let y1 = (((gy + 1) as u32 * h) / GRID_H as u32).clamp(y0 + 1, h);

                let mut sums = [0u64; 3];
                let mut n = 0u64;
                for y in y0..y1 {
                    for x in x0..x1 {
                        n += 1;
                        for (sum, value) in sums.iter_mut().zip(image.get_pixel(x, y).0) {
                            *sum += u64::from(value);
                        }
                    }
                }

                for channel in 0..3 {
                    let reference = if n == 0 { 0.0 } else { sums[channel] as f32 / n as f32 };
                    // Below this a point saw nothing worth scaling, and the
                    // gain it would need is the confetti MAX_GAIN guards.
                    let g = if reference >= target / MAX_GAIN { target / reference } else { 0.0 };
                    gain[(gy * GRID_W + gx) * 3 + channel] = g;
                }
            }
        }
        Self { gain, trailer }
    }

    /// Write the calibration back out in the vendor's own format.
    ///
    /// Byte for byte what [`Shading::parse`] reads, so a file written here is
    /// one the shipped software could read too - including bottom-up rows and
    /// BGR channels in the file, top-down rows and RGB in memory.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((TRAILER + GRID_W * GRID_H * 3) * 4);
        for i in 0..GRID_W * GRID_H * 3 {
            let channel = i % 3;
            let x = (i / 3) % GRID_W;
            let y = i / (GRID_W * 3);
            let source = ((GRID_H - 1 - y) * GRID_W + x) * 3 + (2 - channel);
            out.extend(self.gain[source].to_le_bytes());
        }
        for value in self.trailer {
            out.extend(value.to_le_bytes());
        }
        out
    }
}

/// No geometric crop/resize for a newly measured whole-frame reference.
fn whole_frame_geometry(width: u32, height: u32) -> [f32; TRAILER] {
    [
        0.0,
        0.0,
        width as f32,
        height as f32,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
        1.0,
        1.0,
        0.0,
        0.0,
        width as f32,
        height as f32,
    ]
}

/// Map a pixel coordinate onto the calibration grid.
fn scale(value: u32, size: u32, grid: usize) -> f32 {
    if size <= 1 {
        return 0.0;
    }
    let t = f32::from(u16::try_from(value.min(size - 1)).unwrap_or(u16::MAX))
        / f32::from(u16::try_from(size - 1).unwrap_or(u16::MAX));
    t * (grid - 1) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file whose coefficients encode their own position, so a reader that
    /// transposes, mis-strides or mixes up the channels is caught.
    fn file(make: impl Fn(usize, usize, usize) -> f32) -> Vec<u8> {
        let mut out = Vec::with_capacity((TRAILER + GRID_W * GRID_H * 3) * 4);
        for y in (0..GRID_H).rev() {
            for x in 0..GRID_W {
                // Written BGR, as the file stores it.
                for channel in [2usize, 1, 0] {
                    out.extend(make(x, y, channel).to_le_bytes());
                }
            }
        }
        for value in whole_frame_geometry(2592, 1944) {
            out.extend(value.to_le_bytes());
        }
        out
    }

    fn flat(value: f32) -> Shading {
        Shading::parse(&file(|_, _, _| value)).unwrap()
    }

    #[test]
    fn a_calibration_built_from_a_reference_flattens_that_reference() {
        // The whole point: run the capture it was built from back through the
        // correction and every part of it should land on the target.
        let uneven = image::RgbImage::from_fn(2592, 1944, |x, y| {
            // Bright in the middle, falling off to the corners, the way a
            // strip of LEDs behind a diffuser actually lights a platen.
            let dx = (f32::from(u16::try_from(x).unwrap()) - 1296.0) / 1296.0;
            let dy = (f32::from(u16::try_from(y).unwrap()) - 972.0) / 972.0;
            // A factor of about three corner to centre, which is the order
            // the real platen falls off by.
            let v = (200.0 * (1.0 - 0.33 * (dx * dx + dy * dy))) as u8;
            image::Rgb([v, v, v])
        });

        let shading = Shading::from_reference(&uneven, TARGET_VISIBLE);
        for (x, y) in [(20u32, 20u32), (1296, 972), (2571, 1923), (600, 1500)] {
            let raw = f32::from(uneven.get_pixel(x, y).0[1]);
            let gain = shading.sample(x, y, 2592, 1944, 1).expect("lit everywhere here");
            let corrected = raw * gain;
            assert!(
                // The reference is eight-bit, so the gain it implies is
                // quantised; a couple of levels either way is that and not an
                // error in the arithmetic.
                (corrected - TARGET_VISIBLE).abs() < 5.0,
                "at {x},{y}: {raw} x {gain} = {corrected}, wanted {TARGET_VISIBLE}"
            );
        }
    }

    #[test]
    fn a_reference_that_saw_nothing_is_left_uncalibrated() {
        let dark = image::RgbImage::from_pixel(2592, 1944, image::Rgb([1, 1, 1]));
        let shading = Shading::from_reference(&dark, TARGET_VISIBLE);
        assert_eq!(shading.at(100, 100, 0), None);
    }

    #[test]
    fn a_calibration_survives_a_round_trip_through_the_file_format() {
        let reference = image::RgbImage::from_fn(648, 486, |x, y| {
            image::Rgb([(100 + x % 60) as u8, (110 + y % 50) as u8, (120 + (x + y) % 40) as u8])
        });
        let built = Shading::from_reference(&reference, TARGET_VISIBLE);
        let read_back = Shading::parse(&built.to_bytes()).expect("what we write, we can read");

        for (x, y, c) in [(0usize, 0usize, 0usize), (320, 240, 1), (647, 485, 2), (100, 400, 0)] {
            assert_eq!(read_back.at(x, y, c), built.at(x, y, c), "at {x},{y} channel {c}");
        }
    }

    #[test]
    fn what_is_written_is_the_size_the_shipped_files_are() {
        let built = Shading::from_reference(
            &image::RgbImage::from_pixel(64, 48, image::Rgb([200; 3])),
            240.0,
        );
        assert_eq!(built.to_bytes().len(), 3_779_208);
    }

    #[test]
    fn a_black_point_lands_in_the_dark_mass_not_in_the_document() {
        // A frame as the sensor delivers one: a lot of near-black platen with
        // an unequal floor per channel, and a bright card on top of it.
        let mut histograms = [[0u64; 256]; 3];
        for (channel, floor) in [17usize, 28, 25].into_iter().enumerate() {
            histograms[channel][floor] = 700_000;
            histograms[channel][200] = 300_000;
        }
        assert_eq!(black_point(&histograms, BLACK_POINT), [17, 28, 25]);
    }

    #[test]
    fn an_empty_histogram_has_a_black_point_of_zero() {
        // Nothing to measure means no correction, which is the safe answer.
        assert_eq!(black_point(&[[0u64; 256]; 3], BLACK_POINT), [0, 0, 0]);
    }

    #[test]
    fn a_frame_with_no_dark_mass_gives_back_its_own_darkest_tone() {
        let mut histograms = [[0u64; 256]; 3];
        for histogram in &mut histograms {
            histogram[180] = 1_000_000;
        }
        assert_eq!(black_point(&histograms, BLACK_POINT), [180, 180, 180]);
    }

    #[test]
    fn a_file_of_the_wrong_size_is_refused() {
        assert!(Shading::parse(&[]).is_err());
        assert!(Shading::parse(&[0u8; 100]).is_err());
        let mut short = file(|_, _, _| 1.0);
        short.truncate(short.len() - 4);
        assert!(Shading::parse(&short).is_err());
    }

    #[test]
    fn the_shipped_size_is_the_one_the_format_implies() {
        // 3 779 208 bytes is what both calibration files on the reference unit
        // actually are; this is the arithmetic that pinned the geometry.
        assert_eq!((TRAILER + GRID_W * GRID_H * 3) * 4, 3_779_208);
    }

    #[test]
    fn channels_come_back_in_rgb_though_the_file_holds_bgr() {
        // Distinct per channel, constant across the grid.
        let shading = Shading::parse(&file(|_, _, c| 1.0 + c as f32)).unwrap();
        assert_eq!(shading.at(0, 0, 0), Some(1.0), "red");
        assert_eq!(shading.at(0, 0, 1), Some(2.0), "green");
        assert_eq!(shading.at(0, 0, 2), Some(3.0), "blue");
    }

    #[test]
    fn vendor_bytes_use_bottom_up_rows_and_bgr_channels() {
        // Explicit byte positions, independent of the fixture writer, catch
        // matching but wrong read/write implementations.
        let mut bytes = vec![0u8; (TRAILER + GRID_W * GRID_H * 3) * 4];
        let mut put = |row: usize, values: [f32; 3]| {
            for (channel, value) in values.into_iter().enumerate() {
                let at = (row * GRID_W * 3 + channel) * 4;
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
        };
        put(0, [2.0, 3.0, 4.0]);
        put(GRID_H - 1, [5.0, 6.0, 7.0]);
        let trailer_at = GRID_W * GRID_H * 3 * 4;
        for (i, value) in [
            0.0f32, 39.0, 2517.0, 1918.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.98, 0.987, 0.0,
            39.0, 2517.0, 1918.0,
        ]
        .into_iter()
        .enumerate()
        {
            bytes[trailer_at + i * 4..trailer_at + (i + 1) * 4]
                .copy_from_slice(&value.to_le_bytes());
        }
        let shading = Shading::parse(&bytes).unwrap();
        for (channel, expected) in [7.0, 6.0, 5.0].into_iter().enumerate() {
            assert_eq!(shading.at(0, 0, channel), Some(expected));
        }
        for (channel, expected) in [4.0, 3.0, 2.0].into_iter().enumerate() {
            assert_eq!(shading.at(0, GRID_H - 1, channel), Some(expected));
        }
        assert_eq!(shading.to_bytes(), bytes);
    }

    #[test]
    fn a_coefficient_is_found_at_its_own_position() {
        // Encode x in the value; a wrong stride lands on the wrong column.
        let shading =
            Shading::parse(&file(|x, y, _| 1.0 + (x % 7) as f32 + (y % 3) as f32)).unwrap();
        for (x, y) in [(0, 0), (5, 2), (647, 485), (100, 400)] {
            assert_eq!(
                shading.at(x, y, 0),
                Some(1.0 + (x % 7) as f32 + (y % 3) as f32),
                "at {x},{y}"
            );
        }
    }

    #[test]
    fn a_reading_outside_the_grid_is_no_reading() {
        let shading = flat(1.5);
        assert_eq!(shading.at(GRID_W, 0, 0), None);
        assert_eq!(shading.at(0, GRID_H, 0), None);
        assert_eq!(shading.at(0, 0, 3), None);
    }

    #[test]
    fn an_uncalibrated_pixel_is_no_reading_rather_than_a_huge_one() {
        // What the corners of the real files hold: a pixel that saw nothing.
        assert_eq!(flat(0.0).at(10, 10, 0), None);
        assert_eq!(flat(2517.0).at(10, 10, 0), None);
        assert_eq!(flat(f32::INFINITY).at(10, 10, 0), None);
        assert_eq!(flat(MAX_GAIN).at(10, 10, 0), Some(MAX_GAIN), "the limit itself is usable");
    }

    #[test]
    fn a_flat_calibration_samples_flat_everywhere() {
        let shading = flat(1.25);
        for (x, y) in [(0, 0), (1295, 971), (2591, 1943)] {
            assert_eq!(shading.sample(x, y, 2592, 1944, 1), Some(1.25), "at {x},{y}");
        }
    }

    #[test]
    fn sampling_follows_a_gradient_rather_than_stepping_through_it() {
        // Gain rising with x: a nearest-neighbour reader would return the same
        // value for four pixels running, which is what this rules out.
        let shading = Shading::parse(&file(|x, _, _| 1.0 + x as f32 / 1000.0)).unwrap();
        let mid: Vec<f32> =
            (1000..1008).map(|x| shading.sample(x, 500, 2592, 1944, 0).unwrap()).collect();
        assert!(mid.windows(2).all(|w| w[1] > w[0]), "should rise smoothly, got {mid:?}");
        let steps: Vec<f32> = mid.windows(2).map(|w| w[1] - w[0]).collect();
        let spread = steps.iter().copied().fold(f32::MIN, f32::max)
            - steps.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread < 1e-5, "the steps should be even, got {steps:?}");
    }

    #[test]
    fn the_corners_of_an_image_land_on_the_corners_of_the_grid() {
        let shading =
            Shading::parse(&file(|x, y, _| 1.0 + x as f32 / 10_000.0 + y as f32 / 1_000.0))
                .unwrap();
        let top_left = shading.sample(0, 0, 2592, 1944, 0).unwrap();
        let bottom_right = shading.sample(2591, 1943, 2592, 1944, 0).unwrap();
        assert!((top_left - 1.0).abs() < 1e-5, "got {top_left}");
        let want = 1.0 + 647.0 / 10_000.0 + 485.0 / 1_000.0;
        assert!((bottom_right - want).abs() < 1e-4, "got {bottom_right}, wanted {want}");
    }

    #[test]
    fn an_empty_image_samples_nothing_rather_than_dividing_by_zero() {
        assert_eq!(flat(1.0).sample(0, 0, 0, 0, 0), None);
    }

    #[test]
    fn an_empty_reference_is_uncalibrated_instead_of_panicking() {
        let shading = Shading::from_reference(&image::RgbImage::new(0, 0), TARGET_VISIBLE);
        assert_eq!(shading.at(0, 0, 0), None);
    }

    #[test]
    fn a_pixel_beside_an_uncalibrated_one_is_left_uncalibrated_too() {
        // Smearing a real reading into the dark corner would put a bright
        // fringe exactly where the document finder goes looking.
        let shading = Shading::parse(&file(|x, _, _| if x < 2 { 0.0 } else { 1.5 })).unwrap();
        assert_eq!(shading.sample(0, 0, 2592, 1944, 0), None);
        assert_eq!(shading.sample(2591, 0, 2592, 1944, 0), Some(1.5));
    }
}
