//! Flattening the lighting a capture was actually taken under.
//!
//! The unit ships a `.shd` flat-field calibration and for a while this crate
//! applied it. Measuring what it does to a real capture settled that it must
//! not: the vendor's own white reference, `Calibration_General.jpg`, is
//! uneven by ±15 % across the frame, while a card on this scanner comes off
//! the sensor flat to ±2 % — 208…216 per channel over the whole card. The
//! sensor is already correcting its own lens shading, from the LENC
//! coefficients the EEPROM register tables write into it at the start of
//! every scan, so by the time a frame reaches this code the work is done.
//!
//! Applying the file on top of that does not flatten anything. It pours the
//! vendor's 15 % gradient back in, inverted, and — because each coefficient is
//! `240 / white_reference` and a laminated card is brighter than the matte
//! sheet the reference was taken from — it multiplies the card past the
//! ceiling: measured, 64 % of the document at exactly 255 before any tone
//! stage had run. That is what made the output look washed out, lose the
//! security print and take on a cast no later stage could remove.
//!
//! So the lighting is measured from the frame in front of us instead. The
//! document's own background is a large, near-uniform, near-white surface,
//! which is exactly what a flat-field reference is; fitting a smooth surface
//! through it says how the light falls across *this* capture under *these*
//! settings, with nothing to go stale.
//!
//! Two properties make this safe where the file was not:
//!
//! - It is **exposure preserving**. The fit is divided by its own middle, so
//!   the correction flattens without brightening: no headroom is spent and
//!   nothing is pushed into clipping. Setting the exposure is the levels
//!   stage's job, on a frame that is by then the document alone.
//! - It **cannot be wrong by more than [`MAX_CORRECTION`]**, and a frame with
//!   no usable background is left alone rather than guessed at.

/// Degree of the surface fitted through the background.
///
/// Light from a strip of LEDs behind a diffuser falls off smoothly; a cubic
/// has enough freedom for that and far too little to chase print, a
/// photograph or a fingerprint on the glass.
pub const ORDER: usize = 3;

/// Number of coefficients in a two-dimensional surface of [`ORDER`].
pub const TERMS: usize = (ORDER + 1) * (ORDER + 2) / 2;

/// How far from flat the correction may go, as a fraction either way.
///
/// The residual actually measured on this unit is about 2 %. A limit well
/// above that still lets a genuinely uneven capture be fixed, while making it
/// impossible for a bad fit to do what the `.shd` file did.
pub const MAX_CORRECTION: f32 = 0.35;

/// Percentile of brightness above which a pixel counts as the background.
///
/// The document is mostly its own paper, so a little over half the frame is
/// the surface we want to measure. Print, photograph and engraving all sit
/// below this and are left out of the fit rather than fitted to.
pub const BACKGROUND: f32 = 0.55;

/// Dimmest a fitted background may be and still be treated as a document.
///
/// A frame of unlit platen fits a perfectly good constant surface, and
/// dividing it by itself is arithmetically fine and physically meaningless.
/// Below this there is no document in front of the lamp and the right answer
/// is to leave the frame alone.
pub const MIN_LEVEL: f64 = 32.0;

/// Fewest background pixels worth fitting a surface through.
///
/// Comfortably more than the twenty unknowns three channels of a cubic come
/// to, so the system is never close to under-determined.
pub const MIN_SAMPLES: usize = 4_000;

/// The lighting across one capture, per channel.
#[derive(Debug, Clone)]
pub struct Illumination {
    /// Surface coefficients, one set per channel.
    coefficients: [[f64; TERMS]; 3],
    /// The value each channel's surface is normalised against.
    middle: [f64; 3],
}

/// The polynomial terms at a point, in normalised coordinates.
fn terms(x: f64, y: f64) -> [f64; TERMS] {
    let mut out = [0.0; TERMS];
    let mut n = 0;
    for i in 0..=ORDER {
        for j in 0..=(ORDER - i) {
            out[n] = x.powi(i as i32) * y.powi(j as i32);
            n += 1;
        }
    }
    out
}

/// Map a pixel coordinate to −1…1, which keeps the fit well conditioned.
fn normalise(value: u32, size: u32) -> f64 {
    if size <= 1 {
        return 0.0;
    }
    f64::from(value) / f64::from(size - 1) * 2.0 - 1.0
}

impl Illumination {
    /// Measure the lighting across a capture from its own background.
    ///
    /// Returns `None` when there is not enough background to measure — a frame
    /// that is nearly all print, or one too small to fit through — in which
    /// case the right thing to do is nothing.
    #[must_use]
    pub fn measure(image: &image::RgbImage) -> Option<Self> {
        let (w, h) = image.dimensions();
        let threshold = background_threshold(image)?;

        // Normal equations, accumulated per channel in one pass over the
        // background pixels. Twenty by twenty is small enough to solve
        // directly and the accumulation is what costs anything.
        let mut normal = [[0f64; TERMS]; TERMS];
        let mut rhs = [[0f64; TERMS]; 3];
        let mut samples = 0usize;

        for y in 0..h {
            for x in 0..w {
                let pixel = image.get_pixel(x, y).0;
                if crate::luma(pixel) < threshold {
                    continue;
                }
                samples += 1;
                let t = terms(normalise(x, w), normalise(y, h));
                for (i, ti) in t.iter().enumerate() {
                    for (j, tj) in t.iter().enumerate() {
                        normal[i][j] += ti * tj;
                    }
                    for (channel, row) in rhs.iter_mut().enumerate() {
                        row[i] += ti * f64::from(pixel[channel]);
                    }
                }
            }
        }
        if samples < MIN_SAMPLES {
            return None;
        }

        let mut coefficients = [[0f64; TERMS]; 3];
        for (channel, slot) in coefficients.iter_mut().enumerate() {
            *slot = solve(normal, rhs[channel])?;
        }

        // The middle of each surface, taken at the centre of the frame: the
        // correction is relative to this, so the stage flattens without
        // changing the overall exposure.
        let centre = terms(0.0, 0.0);
        let mut middle = [0f64; 3];
        for (slot, coefficient) in middle.iter_mut().zip(&coefficients) {
            *slot = centre.iter().zip(coefficient).map(|(t, c)| t * c).sum();
            if !slot.is_finite() || *slot < MIN_LEVEL {
                return None;
            }
        }
        Some(Self { coefficients, middle })
    }

    /// The gain that flattens one point, already limited to
    /// [`MAX_CORRECTION`].
    #[must_use]
    pub fn gain(&self, x: u32, y: u32, width: u32, height: u32, channel: usize) -> f32 {
        if channel >= 3 {
            return 1.0;
        }
        let t = terms(normalise(x, width), normalise(y, height));
        let surface: f64 =
            t.iter().zip(&self.coefficients[channel]).map(|(term, c)| term * c).sum();
        if !surface.is_finite() || surface < 1.0 {
            return 1.0;
        }
        let gain = (self.middle[channel] / surface) as f32;
        gain.clamp(1.0 - MAX_CORRECTION, 1.0 + MAX_CORRECTION)
    }

    /// Flatten a capture in place.
    pub fn apply(&self, image: &mut image::RgbImage) {
        let (w, h) = image.dimensions();
        for y in 0..h {
            for x in 0..w {
                let pixel = image.get_pixel_mut(x, y);
                for (channel, value) in pixel.0.iter_mut().enumerate() {
                    let out = f32::from(*value) * self.gain(x, y, w, h, channel);
                    *value = out.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

/// The brightness a pixel has to reach to count as background.
fn background_threshold(image: &image::RgbImage) -> Option<u8> {
    let mut histogram = [0u64; 256];
    for p in image.pixels() {
        histogram[crate::luma(p.0) as usize] += 1;
    }
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return None;
    }
    let want = (total as f32 * BACKGROUND) as u64;
    let mut seen = 0u64;
    for (value, count) in histogram.iter().enumerate() {
        seen += count;
        if seen > want {
            return Some(value as u8);
        }
    }
    None
}

/// Solve a small symmetric system by Gauss–Jordan with partial pivoting.
///
/// Returns `None` if it is singular, which for a capture means the background
/// gave nothing to fit — the same answer as too few samples.
fn solve(mut a: [[f64; TERMS]; TERMS], mut b: [f64; TERMS]) -> Option<[f64; TERMS]> {
    for column in 0..TERMS {
        let pivot = (column..TERMS).max_by(|&i, &j| {
            a[i][column].abs().partial_cmp(&a[j][column].abs()).unwrap_or(std::cmp::Ordering::Equal)
        })?;
        if a[pivot][column].abs() < 1e-9 {
            return None;
        }
        a.swap(column, pivot);
        b.swap(column, pivot);

        let diagonal = a[column][column];
        for value in &mut a[column] {
            *value /= diagonal;
        }
        b[column] /= diagonal;

        for row in 0..TERMS {
            if row == column {
                continue;
            }
            let factor = a[row][column];
            if factor == 0.0 {
                continue;
            }
            let (pivot_row, target_row) = if row < column {
                let (before, after) = a.split_at_mut(column);
                (&after[0], &mut before[row])
            } else {
                let (before, after) = a.split_at_mut(row);
                (&before[column], &mut after[0])
            };
            for (value, p) in target_row.iter_mut().zip(pivot_row) {
                *value -= factor * p;
            }
            b[row] -= factor * b[column];
        }
    }
    b.iter().all(|v| v.is_finite()).then_some(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page lit the way a strip of LEDs lights one: brightest down the
    /// middle, falling off to the sides, with a line of print on it.
    fn lit_page(fall: f32) -> image::RgbImage {
        image::RgbImage::from_fn(600, 400, |x, y| {
            let dx = (f32::from(u16::try_from(x).unwrap()) - 300.0) / 300.0;
            let shade = 1.0 - fall * dx * dx;
            let base = if (120..150).contains(&y) { 40.0 } else { 210.0 };
            let v = (base * shade).round().clamp(0.0, 255.0) as u8;
            image::Rgb([v, v, v])
        })
    }

    #[test]
    fn a_page_lit_unevenly_comes_out_even() {
        let mut page = lit_page(0.25);
        Illumination::measure(&page).expect("a page is mostly background").apply(&mut page);

        // Sample the background across the width; it should now be one value.
        let row: Vec<f32> =
            (20..580).step_by(40).map(|x| f32::from(page.get_pixel(x, 300).0[1])).collect();
        let spread = row.iter().copied().fold(f32::MIN, f32::max)
            - row.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread <= 4.0, "should be flat across the width, got {row:?}");
    }

    #[test]
    fn flattening_does_not_change_the_exposure() {
        // The whole point of dividing by the surface's own middle: the centre
        // of the frame is where the correction is one, so a capture comes out
        // as bright as it went in. The `.shd` path did not have this property
        // and that is what blew the highlights.
        let mut page = lit_page(0.25);
        let before = f32::from(page.get_pixel(300, 300).0[1]);
        Illumination::measure(&page).unwrap().apply(&mut page);
        let after = f32::from(page.get_pixel(300, 300).0[1]);
        assert!((after - before).abs() <= 2.0, "{before} became {after}");
    }

    #[test]
    fn an_already_flat_page_is_left_alone() {
        // What this unit actually delivers. A correction that does nothing to
        // it is the correct answer, and the one the shipped file got wrong.
        let flat = image::RgbImage::from_pixel(600, 400, image::Rgb([212, 215, 213]));
        let measured = Illumination::measure(&flat).expect("all background");
        for (x, y) in [(0u32, 0u32), (300, 200), (599, 399)] {
            for channel in 0..3 {
                let gain = measured.gain(x, y, 600, 400, channel);
                assert!((gain - 1.0).abs() < 0.01, "at {x},{y}.{channel}: {gain}");
            }
        }
    }

    #[test]
    fn the_print_is_not_fitted_to() {
        // The fit must describe the lighting, not the document. A page with a
        // broad dark block on one side would pull a surface fitted through
        // everything; fitted through the background only, it does not.
        let mut page = image::RgbImage::from_pixel(600, 400, image::Rgb([210; 3]));
        for y in 0..400 {
            for x in 0..200 {
                page.put_pixel(x, y, image::Rgb([30; 3]));
            }
        }
        let measured = Illumination::measure(&page).expect("two thirds is background");
        let gain = measured.gain(500, 200, 600, 400, 1);
        assert!((gain - 1.0).abs() < 0.05, "the lit side should need no correction, got {gain}");
    }

    #[test]
    fn a_correction_can_never_run_away() {
        let page = lit_page(0.9);
        let measured = Illumination::measure(&page).expect("still mostly background");
        for x in (0..600).step_by(25) {
            let gain = measured.gain(x, 200, 600, 400, 0);
            assert!(
                (1.0 - MAX_CORRECTION..=1.0 + MAX_CORRECTION).contains(&gain),
                "at {x}: {gain}"
            );
        }
    }

    #[test]
    fn a_frame_with_no_background_is_not_measured() {
        let dark = image::RgbImage::from_pixel(600, 400, image::Rgb([3, 3, 3]));
        // Everything is the same value, so there is no surface to fit and the
        // honest answer is to leave the frame alone.
        assert!(Illumination::measure(&dark).is_none());
    }

    #[test]
    fn a_frame_too_small_to_fit_is_not_measured() {
        let tiny = image::RgbImage::from_pixel(20, 20, image::Rgb([200; 3]));
        assert!(Illumination::measure(&tiny).is_none());
    }

    #[test]
    fn the_term_count_matches_the_order() {
        assert_eq!(terms(0.5, 0.25).len(), TERMS);
        assert_eq!(TERMS, 10);
    }
}
