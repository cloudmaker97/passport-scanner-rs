//! Putting a document's tones where a document's tones belong.
//!
//! Three faults at once, because on this scanner they are one problem:
//!
//! - **Exposure.** A card lit by its own lamp arrives as a narrow band of
//!   bright tones - paper around 210, print not far below - and the device
//!   carries no exposure control to open that up with. Here the paper is put
//!   at [`PAPER`] and the print at black.
//! - **A cast at the white end.** Neither lamp is white, so the paper comes
//!   off the sensor tinted. Each channel is stretched to its own ends, which
//!   sets the white point per channel and takes that out.
//! - **A cast in between.** Stray light inside the housing adds a little of
//!   the lamp's own colour to everything, and being additive it tells for far
//!   more against a dark tone than a light one: measured on this unit, a
//!   laser-engraved portrait came back twenty levels green while the paper it
//!   sat on was neutral to within three. Anchoring the two ends cannot reach
//!   that, so a gamma per channel is fitted through the midtones as well.
//!
//! The endpoints are read from an inset box, so slivers of platen left in the
//! corners by a card lying askew are not mistaken for the document's own
//! shadow.

use crate::{INSET, Stage, for_inset, luma};
use image::RgbImage;

/// What a document's paper is put at.
///
/// Short of 255 on purpose. A document's background is not the brightest
/// thing it carries - a hologram, a laminate highlight or a reflective
/// security print sits above it - and leaving a little room over the paper is
/// what keeps those from flattening into one white shape.
pub const PAPER: f32 = 252.0;

/// Furthest a channel's midtones may be bent to neutralise them.
///
/// A limit rather than a free fit: a genuinely coloured document should come
/// out coloured, and only a cast - small, and the same everywhere - should be
/// taken off.
pub const MAX_NEUTRAL_GAMMA: f32 = 1.35;

/// Darkest a pixel may be and still count towards the midtone fit.
const LOW: u8 = 40;
/// Brightest.
const HIGH: u8 = 215;
/// Widest spread between a pixel's channels for it to count as grey.
const SPREAD: u8 = 60;
/// Fewest such pixels worth fitting a curve through.
const ENOUGH: u64 = 2_000;
/// Narrowest usable gap between the two endpoints of a channel.
const MIN_SPAN: f32 = 16.0;

/// The tone treatment one document gets.
#[derive(Debug, Clone, Copy)]
pub struct DocumentTone {
    /// Fraction of the darkest pixels allowed to clip to black.
    pub shadow: f32,
    /// Fraction of the brightest pixels allowed to clip to white.
    pub highlight: f32,
    /// Fraction of each edge left out when measuring. See [`INSET`].
    pub inset: f32,
    /// Take the colour out of the midtones as well as out of the two ends.
    pub neutralise: bool,
}

impl Default for DocumentTone {
    fn default() -> Self {
        // A card is mostly its own background, so the print is a small tail
        // and the top is cut very little - cut more and the print goes too.
        Self { shadow: 0.003, highlight: 0.005, inset: INSET, neutralise: true }
    }
}

impl DocumentTone {
    /// The treatment a document under the visible lamp gets.
    #[must_use]
    pub fn for_documents() -> Self {
        Self::default()
    }

    /// The same under infrared, where there is no colour to balance.
    ///
    /// The engraving absorbs infrared and almost nothing else does, so the
    /// blue channel carries little but sensor noise. Fitting a curve through
    /// that is fitting a curve through noise, so under infrared the midtones
    /// are left where they are.
    #[must_use]
    pub fn for_infrared() -> Self {
        Self { neutralise: false, ..Self::default() }
    }

    /// Where each channel's black and white points sit.
    fn endpoints(&self, image: &RgbImage) -> [(f32, f32); 3] {
        let mut histograms = [[0u64; 256]; 3];
        for_inset(image, self.inset, |p| {
            for (histogram, value) in histograms.iter_mut().zip(p) {
                histogram[value as usize] += 1;
            }
        });

        let mut out = [(0.0, 255.0); 3];
        for (slot, histogram) in out.iter_mut().zip(&histograms) {
            let total: u64 = histogram.iter().sum();
            if total == 0 {
                continue;
            }
            let black = percentile(histogram, total, self.shadow);
            let white = percentile(histogram, total, 1.0 - self.highlight);
            // Collapsed endpoints would turn the frame into two tones. A
            // channel with nothing to stretch is left as it is.
            if white - black >= MIN_SPAN {
                *slot = (black, white);
            }
        }
        out
    }

    /// A gamma per channel that takes the colour out of the midtones.
    ///
    /// Measured over the pixels that ought to be neutral and are not quite:
    /// those already close to grey, away from both ends. Paper and print are
    /// excluded by the luminance window and anything genuinely coloured - a
    /// blue crest, a green guilloche - by the saturation one, so what is left
    /// is the cast and only the cast.
    ///
    /// Returns ones when there is too little to go on, which leaves the frame
    /// as it is rather than bending it on the strength of a few pixels.
    #[must_use]
    pub fn midtone_gamma(image: &RgbImage) -> [f32; 3] {
        let mut sums = [0f64; 3];
        let mut grey = 0f64;
        let mut n = 0u64;

        for p in image.pixels() {
            let l = luma(p.0);
            if !(LOW..=HIGH).contains(&l) {
                continue;
            }
            let high = p.0.iter().copied().max().unwrap_or(0);
            let low = p.0.iter().copied().min().unwrap_or(0);
            if high.saturating_sub(low) > SPREAD {
                continue;
            }
            n += 1;
            grey += f64::from(l);
            for (sum, value) in sums.iter_mut().zip(p.0) {
                *sum += f64::from(value);
            }
        }
        if n < ENOUGH {
            return [1.0; 3];
        }

        let want = (grey / n as f64 / 255.0).clamp(1e-3, 0.999);
        sums.map(|sum| {
            let have = (sum / n as f64 / 255.0).clamp(1e-3, 0.999);
            // have^gamma = want, both as fractions of full scale, so black
            // and white stay where they are and only the middle moves.
            let gamma = (want.ln() / have.ln()) as f32;
            if gamma.is_finite() {
                gamma.clamp(1.0 / MAX_NEUTRAL_GAMMA, MAX_NEUTRAL_GAMMA)
            } else {
                1.0
            }
        })
    }
}

/// The value `fraction` of a histogram's pixels sit at or below.
fn percentile(histogram: &[u64; 256], total: u64, fraction: f32) -> f32 {
    let want = (total as f32 * fraction.clamp(0.0, 1.0)) as u64;
    let mut seen = 0u64;
    for (value, count) in histogram.iter().enumerate() {
        seen += count;
        if seen > want {
            return value as f32;
        }
    }
    255.0
}

/// Apply one lookup table per channel.
fn map(image: &mut RgbImage, tables: &[[u8; 256]; 3]) {
    for pixel in image.pixels_mut() {
        for (value, table) in pixel.0.iter_mut().zip(tables) {
            *value = table[*value as usize];
        }
    }
}

impl Stage for DocumentTone {
    fn name(&self) -> &str {
        "document tone"
    }

    fn apply(&self, image: &mut RgbImage) {
        let mut stretch = [[0u8; 256]; 3];
        for (table, &(black, white)) in stretch.iter_mut().zip(&self.endpoints(image)) {
            for (value, slot) in table.iter_mut().enumerate() {
                let t = ((value as f32 - black) / (white - black)).clamp(0.0, 1.0);
                *slot = (t * PAPER).round().clamp(0.0, 255.0) as u8;
            }
        }
        map(image, &stretch);

        if !self.neutralise {
            return;
        }
        let gammas = Self::midtone_gamma(image);
        if gammas.iter().all(|g| (g - 1.0).abs() < 0.01) {
            return;
        }
        let mut curves = [[0u8; 256]; 3];
        for (table, &gamma) in curves.iter_mut().zip(&gammas) {
            for (value, slot) in table.iter_mut().enumerate() {
                let t = (value as f32 / 255.0).powf(gamma);
                *slot = (t * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
        map(image, &curves);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    /// A card as the sensor delivers one: a narrow band of bright tones with
    /// a green cast that grows towards the dark end, which is what stray
    /// light inside the housing does.
    fn card(cast: i32) -> RgbImage {
        RgbImage::from_fn(400, 300, |_, y| {
            let base = if y % 20 < 3 { 90 } else { 210 };
            // Additive, so it counts for more against the print than the
            // paper - exactly the fault this stage exists to remove.
            let green = (base + cast).clamp(0, 255) as u8;
            Rgb([base as u8, green, base as u8])
        })
    }

    #[test]
    fn the_paper_ends_up_near_white_and_the_print_near_black() {
        let mut image = card(0);
        DocumentTone::for_documents().apply(&mut image);
        let paper = image.get_pixel(200, 10).0;
        let print = image.get_pixel(200, 1).0;
        assert!(paper[0] >= 240, "paper landed at {paper:?}");
        assert!(print[0] <= 20, "print landed at {print:?}");
    }

    #[test]
    fn nothing_is_pushed_past_the_ceiling() {
        // The fault this whole rewrite is about: the old path put 64 % of a
        // document at exactly 255 before any tone stage had run.
        let mut image = card(0);
        DocumentTone::for_documents().apply(&mut image);
        let clipped = image.pixels().filter(|p| p.0.iter().any(|v| *v >= 254)).count();
        assert!(
            clipped * 100 < image.pixels().len(),
            "{clipped} of {} pixels clipped",
            image.pixels().len()
        );
    }

    #[test]
    fn a_cast_that_sits_in_the_midtones_is_taken_out() {
        let mut image = card(22);
        let before = image.get_pixel(200, 1).0;
        assert!(before[1] > before[0] + 15, "the fixture should start off green");
        DocumentTone::for_documents().apply(&mut image);
        let after = image.get_pixel(200, 1).0;
        assert!(
            i32::from(after[1]) - i32::from(after[0]) < 12,
            "the print should come out near neutral, got {after:?}"
        );
    }

    #[test]
    fn infrared_leaves_the_midtones_alone() {
        // Under infrared the blue channel is mostly noise, and a curve fitted
        // through noise is worse than no curve.
        assert!(!DocumentTone::for_infrared().neutralise);
        assert!(DocumentTone::for_documents().neutralise);
    }

    #[test]
    fn too_few_grey_pixels_means_no_curve() {
        let flat = RgbImage::from_pixel(10, 10, Rgb([120, 130, 125]));
        assert_eq!(DocumentTone::midtone_gamma(&flat), [1.0; 3]);
    }

    #[test]
    fn a_strongly_coloured_frame_is_not_bleached() {
        // Saturated pixels are left out of the fit, so a blue crest stays
        // blue rather than being averaged towards grey.
        let blue = RgbImage::from_pixel(200, 200, Rgb([40, 70, 200]));
        assert_eq!(DocumentTone::midtone_gamma(&blue), [1.0; 3]);
    }

    #[test]
    fn the_correction_is_bounded() {
        // However wrong the frame, the curve cannot run away with it.
        let green = RgbImage::from_fn(300, 300, |x, _| {
            let v = (x % 180) as u8 + 40;
            Rgb([v / 2, v, v / 2])
        });
        for gamma in DocumentTone::midtone_gamma(&green) {
            assert!(
                (1.0 / MAX_NEUTRAL_GAMMA..=MAX_NEUTRAL_GAMMA).contains(&gamma),
                "gamma {gamma} is outside the limit"
            );
        }
    }

    #[test]
    fn an_empty_channel_is_left_alone_rather_than_amplified() {
        let flat = RgbImage::from_pixel(100, 100, Rgb([200, 200, 200]));
        let ends = DocumentTone::for_documents().endpoints(&flat);
        assert_eq!(ends, [(0.0, 255.0); 3], "a flat channel has nothing to stretch");
    }
}
