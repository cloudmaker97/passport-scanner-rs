//! Taking the colour cast off a capture.
//!
//! Neither lamp is white. The visible one is a warm LED behind a diffuser and
//! the infrared one is a single wavelength, so a card comes off the sensor
//! tinted — green under one, red under the other. Nothing on the device
//! corrects for it: the sensor's own white-balance registers are written from
//! the calibration table and left alone.
//!
//! The correction is anchored on the card's white, not on its average. Grey
//! world is the usual trick and it is wrong here: a German identity card
//! carries a large green guilloche, so its average really is green, and
//! averaging to grey swings the whole card pink. What is reliably neutral is
//! the paper the print sits on, which is the brightest thing in the frame.

/// How far a single channel may be pushed.
///
/// Under infrared there is almost no blue to work with, and an unbounded gain
/// would turn its sensor noise into coloured confetti. Clamping leaves a
/// little cast behind in that case, which is the better failure.
pub const MAX_GAIN: f32 = 2.0;

/// Fraction of the brightest pixels to look past when reading a white point.
///
/// A speck of specular glare off the laminate is not the card's white.
pub const HIGHLIGHT: f32 = 0.002;

/// The value `fraction` of the pixels sit at or above.
///
/// Returns `None` for an empty histogram.
#[must_use]
pub fn highlight(histogram: &[u64; 256], fraction: f32) -> Option<u8> {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return None;
    }
    let want = (total as f32 * fraction) as u64;
    let mut seen = 0u64;
    for (value, count) in histogram.iter().enumerate().rev() {
        seen += count;
        if seen > want {
            return Some(value as u8);
        }
    }
    Some(0)
}

/// Per-channel multipliers that bring three white points onto one.
///
/// They are equalised to their average rather than to the brightest of them,
/// so the correction cannot push the strongest channel into clipping.
///
/// Returns `None` when a channel has no white point to scale.
#[must_use]
pub fn gains(whites: [f32; 3]) -> Option<[f32; 3]> {
    if whites.iter().any(|w| *w <= 0.0) {
        return None;
    }
    let target = (whites[0] + whites[1] + whites[2]) / 3.0;
    Some(whites.map(|w| (target / w).clamp(1.0 / MAX_GAIN, MAX_GAIN)))
}

/// Build the three lookup tables for one set of gains.
#[must_use]
pub fn tables(gains: [f32; 3]) -> [[u8; 256]; 3] {
    gains.map(|gain| {
        let mut table = [0u8; 256];
        for (value, slot) in table.iter_mut().enumerate() {
            *slot = (value as f32 * gain).round().clamp(0.0, 255.0) as u8;
        }
        table
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_neutral_image_is_left_as_it_is() {
        let g = gains([210.0, 210.0, 210.0]).unwrap();
        for gain in g {
            assert!((gain - 1.0).abs() < 1e-6, "a grey card should not be touched, got {gain}");
        }
    }

    #[test]
    fn a_green_cast_is_pulled_back_toward_grey() {
        // The visible lamp: the paper reads green ahead of the rest.
        let g = gains([196.0, 228.0, 206.0]).unwrap();
        assert!(g[1] < 1.0, "the strong channel should come down");
        assert!(g[0] > 1.0, "the weak one should come up");

        let after = [196.0 * g[0], 228.0 * g[1], 206.0 * g[2]];
        let spread = |v: &[f32; 3]| {
            v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min)
        };
        assert!(spread(&after) < spread(&[196.0, 228.0, 206.0]) / 4.0);
    }

    #[test]
    fn a_channel_with_almost_nothing_in_it_is_not_blown_up() {
        // Infrared: blue is noise, and an unbounded gain would amplify it.
        let g = gains([220.0, 140.0, 6.0]).unwrap();
        assert!(g[2] <= MAX_GAIN, "the empty channel was pushed to {}", g[2]);
    }

    #[test]
    fn an_empty_channel_yields_nothing_rather_than_a_division_by_zero() {
        assert_eq!(gains([100.0, 0.0, 100.0]), None);
    }

    #[test]
    fn a_white_point_looks_past_a_speck_of_glare() {
        let mut h = [0u64; 256];
        h[200] = 100_000;
        h[255] = 50; // glare off the laminate
        assert_eq!(highlight(&h, HIGHLIGHT), Some(200));
    }

    #[test]
    fn an_empty_histogram_has_no_white_point() {
        assert_eq!(highlight(&[0u64; 256], HIGHLIGHT), None);
    }

    #[test]
    fn the_tables_are_monotonic_and_clamp_at_the_top() {
        let [r, _, _] = tables([1.8, 1.0, 1.0]);
        assert_eq!(r[0], 0);
        assert_eq!(r[255], 255, "a gain above one must clamp, not wrap");
        assert!(r.windows(2).all(|w| w[1] >= w[0]));
    }
}
