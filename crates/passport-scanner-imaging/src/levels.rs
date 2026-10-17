//! Stretching a capture's tones so what is on the card can be read.
//!
//! Pure arithmetic over a histogram, so the part that decides how far to
//! stretch is testable without an image or a camera.

/// Where to cut the histogram, as a fraction of the pixels counted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cuts {
    /// Fraction of the darkest pixels allowed to clip to black.
    pub shadow: f32,
    /// Fraction of the brightest pixels allowed to clip to white.
    pub highlight: f32,
}

impl Default for Cuts {
    fn default() -> Self {
        // A card is mostly its own background, so the ink is a small tail. Cut
        // little at the top or the print goes with it.
        Self { shadow: 0.004, highlight: 0.0015 }
    }
}

/// The two points a stretch maps to black and white.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlackWhite {
    /// Value that becomes 0.
    pub black: u8,
    /// Value that becomes 255.
    pub white: u8,
}

/// Split a histogram into its dark and light halves.
///
/// A capture is two things at once: the card, lit by the lamp, and the platen
/// around it, which is near black. Taken together the dark surround pins the
/// shadow point at nothing and the stretch does almost no work on the card.
/// This finds where the two separate, by Otsu — the threshold that minimises
/// the variance inside each half — so the points can be taken from the lit
/// side alone.
///
/// Returns `None` for a histogram with no pixels in it.
#[must_use]
pub fn split(histogram: &[u64; 256]) -> Option<u8> {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return None;
    }
    let sum: f64 = histogram.iter().enumerate().map(|(v, c)| v as f64 * *c as f64).sum();

    let (mut below, mut weight_below) = (0f64, 0u64);
    let (mut best, mut best_variance) = (0u8, -1f64);
    for (value, count) in histogram.iter().enumerate() {
        weight_below += count;
        if weight_below == 0 {
            continue;
        }
        let weight_above = total - weight_below;
        if weight_above == 0 {
            break;
        }
        below += value as f64 * *count as f64;
        let mean_below = below / weight_below as f64;
        let mean_above = (sum - below) / weight_above as f64;
        // Between-class variance; the threshold that maximises it is the one
        // that separates the two populations best.
        let variance =
            weight_below as f64 * weight_above as f64 * (mean_below - mean_above).powi(2);
        if variance > best_variance {
            best_variance = variance;
            best = value as u8;
        }
    }
    Some(best)
}

/// Mean of everything at or below `threshold`, or `None` if nothing is.
#[must_use]
pub fn mean_below(histogram: &[u64; 256], threshold: u8) -> Option<(f64, u64)> {
    let slice = &histogram[..=threshold as usize];
    let count: u64 = slice.iter().sum();
    (count > 0).then(|| {
        let sum: f64 = slice.iter().enumerate().map(|(v, c)| v as f64 * *c as f64).sum();
        (sum / count as f64, count)
    })
}

/// Mean of everything above `threshold`, or `None` if nothing is.
#[must_use]
pub fn mean_above(histogram: &[u64; 256], threshold: u8) -> Option<(f64, u64)> {
    let from = threshold as usize + 1;
    let slice = histogram.get(from..)?;
    let count: u64 = slice.iter().sum();
    (count > 0).then(|| {
        let sum: f64 = slice.iter().enumerate().map(|(v, c)| (v + from) as f64 * *c as f64).sum();
        (sum / count as f64, count)
    })
}

/// How much darker than the lit side the platen has to be.
///
/// **Relative, not absolute, and that matters.** This was a fixed ceiling of
/// 64 until the lamp was given time to come up before the first scan: at full
/// output the platen reads 76 here, and every capture in a run of twenty was
/// then handed back uncropped because a perfectly good platen had been
/// declared too bright to be one. What makes a platen a platen is that it is
/// far darker than the document on it, which does not depend on how bright
/// the lamp is.
pub const UNLIT_RATIO: f64 = 0.55;

/// Below this share of the frame the dark side is a detail, not a surround.
pub const UNLIT_SHARE: f64 = 0.05;

/// Find the unlit platen under a capture, if there is one.
///
/// [`split`] will separate *something* in any image — given a card filling the
/// frame it would happily split the print from the paper, which is the one
/// thing that must not happen here. So the split is only accepted when the
/// dark side looks like what it is supposed to be: far darker than the lit
/// side, and a real part of the frame rather than a few shadowed pixels.
#[must_use]
pub fn platen(histogram: &[u64; 256]) -> Option<u8> {
    let threshold = split(histogram)?;
    let total: u64 = histogram.iter().sum();
    let (dark, count) = mean_below(histogram, threshold)?;
    let (lit, _) = mean_above(histogram, threshold)?;
    (dark < lit * UNLIT_RATIO && count as f64 >= total as f64 * UNLIT_SHARE).then_some(threshold)
}

/// Drop everything at or below `threshold` from a histogram.
///
/// What is left is the lit side, which is what a document scan is actually of.
#[must_use]
pub fn above(histogram: &[u64; 256], threshold: u8) -> [u64; 256] {
    let mut out = *histogram;
    for slot in out.iter_mut().take(threshold as usize + 1) {
        *slot = 0;
    }
    out
}

/// Find the stretch points in a 256-bin histogram.
///
/// Returns `None` when there is nothing to stretch — an empty histogram, or one
/// whose points would collapse onto each other.
#[must_use]
pub fn black_white(histogram: &[u64; 256], cuts: Cuts) -> Option<BlackWhite> {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return None;
    }

    let want_low = (total as f32 * cuts.shadow) as u64;
    let want_high = (total as f32 * cuts.highlight) as u64;

    let mut seen = 0u64;
    let mut black = 0u8;
    for (value, count) in histogram.iter().enumerate() {
        seen += count;
        if seen > want_low {
            black = value as u8;
            break;
        }
    }

    seen = 0;
    let mut white = 255u8;
    for (value, count) in histogram.iter().enumerate().rev() {
        seen += count;
        if seen > want_high {
            white = value as u8;
            break;
        }
    }

    // A stretch needs room to work in; collapsing points would amplify noise
    // into a two-tone image.
    (white.saturating_sub(black) >= 16).then_some(BlackWhite { black, white })
}

/// Build the 256-entry lookup table for one stretch.
///
/// `gamma` below one lifts the midtones, which is what brings faint engraving
/// up without pushing the card's own white any further.
#[must_use]
pub fn table(points: BlackWhite, gamma: f32) -> [u8; 256] {
    let mut out = [0u8; 256];
    let black = f32::from(points.black);
    let span = f32::from(points.white) - black;
    for (value, slot) in out.iter_mut().enumerate() {
        let t = ((value as f32 - black) / span).clamp(0.0, 1.0);
        let shaped = if (gamma - 1.0).abs() < f32::EPSILON { t } else { t.powf(gamma) };
        *slot = (shaped * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A histogram shaped like a card: a big peak of background near white, a
    /// small tail of ink below it.
    fn card() -> [u64; 256] {
        let mut h = [0u64; 256];
        h[195..=215].fill(50_000);
        h[140..=180].fill(400);
        h
    }

    #[test]
    fn a_bright_platen_under_a_warm_lamp_is_still_a_platen() {
        // The regression that cost a whole run of twenty scans. With the lamp
        // given time to come up the platen reads around 76 rather than around
        // 30; under the old fixed ceiling of 64 it stopped counting as a
        // platen and every capture came back uncropped.
        let mut h = [0u64; 256];
        h[70..=82].fill(60_000); // platen, lit lamp
        h[200..=215].fill(40_000); // the card on it
        let threshold = platen(&h).expect("far darker than the card is a platen");
        assert!((82..200).contains(&usize::from(threshold)), "split landed at {threshold}");
    }

    #[test]
    fn the_points_bracket_the_card() {
        let p = black_white(&card(), Cuts::default()).expect("a card has room to stretch");
        assert!(p.black >= 140 && p.black <= 196, "black landed at {}", p.black);
        assert!(p.white >= 210, "white landed at {}", p.white);
    }

    #[test]
    fn a_stretch_puts_the_background_near_white_and_the_ink_far_below() {
        let p = black_white(&card(), Cuts::default()).unwrap();
        let t = table(p, 1.0);
        assert!(t[205] > 200, "the card's own background should stay bright");
        assert!(t[160] < 90, "the ink should drop well away from it");
    }

    #[test]
    fn an_empty_histogram_yields_nothing_rather_than_a_division_by_zero() {
        assert_eq!(black_white(&[0u64; 256], Cuts::default()), None);
    }

    #[test]
    fn a_flat_image_is_left_alone() {
        // Everything one value: stretching it would turn sensor noise into
        // violent banding.
        let mut h = [0u64; 256];
        h[200] = 1_000_000;
        assert_eq!(black_white(&h, Cuts::default()), None);
    }

    #[test]
    fn the_table_is_monotonic_and_spans_the_range() {
        let t = table(BlackWhite { black: 40, white: 220 }, 1.0);
        assert_eq!(t[0], 0);
        assert_eq!(t[255], 255);
        assert!(t.windows(2).all(|w| w[1] >= w[0]), "a tone curve must not fold back");
    }

    /// What the sensor really delivers: a dark platen around a lit card.
    fn card_on_a_platen() -> [u64; 256] {
        let mut h = card();
        h[0..=30].fill(120_000);
        h
    }

    #[test]
    fn the_split_falls_between_the_platen_and_the_card() {
        let t = split(&card_on_a_platen()).expect("a populated histogram has a split");
        // Every threshold from the top of the platen band to the bottom of the
        // card separates the two equally well, so the first one wins. The
        // threshold is inclusive, which is what `above` expects.
        assert!((30..140).contains(&t), "the split landed at {t}");
    }

    #[test]
    fn an_empty_histogram_has_no_split() {
        assert_eq!(split(&[0u64; 256]), None);
    }

    #[test]
    fn a_card_filling_the_frame_has_no_platen_to_find() {
        // The dangerous case: without the guard, Otsu would split the print
        // from the paper and the stretch would then measure the paper alone.
        assert_eq!(platen(&card()), None);
    }

    #[test]
    fn a_card_on_a_platen_has_one() {
        let t = platen(&card_on_a_platen()).expect("a dark surround should be found");
        assert!((30..140).contains(&t), "the platen split landed at {t}");
    }

    #[test]
    fn a_few_shadowed_pixels_are_not_a_platen() {
        let mut h = card();
        h[5] = 200; // a speck, far below the five per cent share
        assert_eq!(platen(&h), None);
    }

    #[test]
    fn the_dark_surround_would_otherwise_swallow_the_stretch() {
        // Straight at the whole frame, the platen pins the shadow point at the
        // bottom of the range and the card is barely touched.
        let whole = black_white(&card_on_a_platen(), Cuts::default()).unwrap();
        assert!(whole.black < 40, "the platen pinned black at {}", whole.black);

        // Restricted to the lit side, the points bracket the card itself.
        let t = platen(&card_on_a_platen()).unwrap();
        let lit = black_white(&above(&card_on_a_platen(), t), Cuts::default()).unwrap();
        assert!(lit.black >= 140, "black should land on the ink, got {}", lit.black);
        assert!(lit.white >= 210, "white should land on the card, got {}", lit.white);
    }

    #[test]
    fn gamma_below_one_lifts_the_midtones() {
        let points = BlackWhite { black: 0, white: 255 };
        let straight = table(points, 1.0);
        let lifted = table(points, 0.7);
        assert!(lifted[128] > straight[128]);
        assert_eq!(lifted[0], 0, "the ends stay put");
        assert_eq!(lifted[255], 255);
    }
}
