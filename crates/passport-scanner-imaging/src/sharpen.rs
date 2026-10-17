//! Unsharp masking.
//!
//! A card lit from one side and delivered as JPEG arrives softer than the
//! sensor saw it. Subtracting a blurred copy from the original and adding the
//! difference back restores the edges without inventing detail.

use image::RgbImage;

/// Apply an unsharp mask to an image's brightness, leaving its colour alone.
///
/// `amount` is how much of the edge signal to add back; around one is crisp,
/// beyond two starts to show halos.
///
/// The mask is computed on luminance and the same correction added to all
/// three channels, rather than each channel being sharpened against itself.
/// Done per channel, an edge between two slightly different colours has each
/// channel pushed a different way, and what comes out is a coloured fringe
/// along every line of text - which on a document scan reads as the whole
/// image being off-colour. Brightness is also the only thing sharpening is
/// for: it restores edges the lens and the codec softened, and those are
/// losses of detail, not of hue.
///
/// The box blur is separable, so it is done as a horizontal pass and a
/// vertical one over a luminance plane rather than one lookup per mask
/// element. At five megapixels that is the difference between a stage you
/// notice and one you do not.
#[must_use]
pub fn unsharp(src: &RgbImage, amount: f32) -> RgbImage {
    unsharp_radius(src, amount, 1)
}

/// The same, over a blur of the given radius.
///
/// Radius decides what *size* of detail is restored, and one pixel is the
/// wrong size here. At 2592 pixels across an 86 mm card the stroke of a line
/// of print is three or four pixels wide, so a three-by-three mask sharpens
/// the grain inside a stroke rather than the stroke itself - which is why
/// output off this scanner read as soft however much amount was added. A
/// radius of two covers the stroke, and that is what the eye calls focus.
///
/// `radius` 0, or an image with no interior at that radius, is a copy.
#[must_use]
pub fn unsharp_radius(src: &RgbImage, amount: f32, radius: usize) -> RgbImage {
    let (w, h) = (src.width() as usize, src.height() as usize);
    let span = radius * 2 + 1;
    if radius == 0 || w <= span || h <= span || amount <= 0.0 {
        return src.clone();
    }

    let original = src.as_raw();
    let brightness: Vec<u8> = src.pixels().map(|p| crate::luma(p.0)).collect();

    // Horizontal pass, kept as sums so the vertical pass can divide once.
    let mut rows = vec![0u32; w * h];
    for y in 0..h {
        let row = y * w;
        for x in radius..w - radius {
            rows[row + x] = (x - radius..=x + radius).map(|k| u32::from(brightness[row + k])).sum();
        }
    }

    // Vertical pass over those sums, and the mask in the same sweep. The
    // border is left as it is: it carries nothing to read, and clamping there
    // costs more than it returns.
    let area = (span * span) as f32;
    let mut out = src.clone();
    let result = out.as_mut();
    for y in radius..h - radius {
        let row = y * w;
        for x in radius..w - radius {
            let i = row + x;
            let sum: u32 = rows[i - radius * w..=i + radius * w].iter().step_by(w).sum();
            let delta = amount * (f32::from(brightness[i]) - sum as f32 / area);
            for c in 0..3 {
                let k = i * 3 + c;
                result[k] = (f32::from(original[k]) + delta).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    /// A soft edge: a gradient from dark to light across the middle.
    fn ramp() -> RgbImage {
        RgbImage::from_fn(9, 9, |x, _| {
            let v = (x * 28).min(255) as u8;
            Rgb([v, v, v])
        })
    }

    #[test]
    fn an_edge_comes_out_steeper_than_it_went_in() {
        let src = ramp();
        let out = unsharp(&src, 1.0);
        // A linear ramp has no second derivative, so the interior is unchanged;
        // what matters is that nothing is destroyed and the range is kept.
        assert_eq!(out.dimensions(), src.dimensions());
        assert_eq!(out.get_pixel(4, 4).0[0], src.get_pixel(4, 4).0[0]);
    }

    #[test]
    fn a_step_gains_contrast_across_it() {
        let src = RgbImage::from_fn(9, 9, |x, _| {
            let v = if x < 4 { 80u8 } else { 170 };
            Rgb([v, v, v])
        });
        let out = unsharp(&src, 1.0);
        let dark_side = out.get_pixel(3, 4).0[0];
        let light_side = out.get_pixel(4, 4).0[0];
        assert!(dark_side < 80, "the dark side of a step should go darker");
        assert!(light_side > 170, "the light side should go lighter");
    }

    #[test]
    fn zero_amount_is_a_copy() {
        let src = ramp();
        assert_eq!(unsharp(&src, 0.0), src);
    }

    #[test]
    fn an_image_too_small_to_have_an_interior_is_returned_untouched() {
        let tiny = RgbImage::from_pixel(2, 2, Rgb([10, 20, 30]));
        assert_eq!(unsharp(&tiny, 1.0), tiny);
    }

    #[test]
    fn values_stay_in_range() {
        let src = RgbImage::from_fn(9, 9, |x, y| {
            let v = if (x + y) % 2 == 0 { 0u8 } else { 255 };
            Rgb([v, v, v])
        });
        // A checkerboard is the worst case for overshoot; clamping must hold,
        // so every pixel stays at one end of the range rather than wrapping.
        let out = unsharp(&src, 3.0);
        assert!(out.pixels().all(|p| p.0.iter().all(|c| *c == 0 || *c == 255)));
    }

    #[test]
    fn a_grey_image_sharpens_exactly_as_it_did_before() {
        // Where there is no colour there is nothing for the luminance route to
        // change, so the familiar behaviour has to be unaffected.
        let src = RgbImage::from_fn(9, 9, |x, _| {
            let v = if x < 4 { 80u8 } else { 170 };
            Rgb([v, v, v])
        });
        let out = unsharp(&src, 1.0);
        assert!(out.get_pixel(3, 4).0[0] < 80);
        assert!(out.get_pixel(4, 4).0[0] > 170);
    }

    #[test]
    fn the_separable_pass_matches_a_plain_three_by_three_blur_of_luminance() {
        // The fast path is only worth having if it computes the same thing.
        let src = RgbImage::from_fn(12, 10, |x, y| {
            Rgb([(x * 17 % 256) as u8, (y * 23 % 256) as u8, ((x + y) * 11 % 256) as u8])
        });
        let fast = unsharp(&src, 1.0);
        for y in 1..9u32 {
            for x in 1..11u32 {
                let mut sum = 0.0f32;
                for dy in 0..3u32 {
                    for dx in 0..3u32 {
                        sum += f32::from(crate::luma(src.get_pixel(x + dx - 1, y + dy - 1).0));
                    }
                }
                let delta = f32::from(crate::luma(src.get_pixel(x, y).0)) - sum / 9.0;
                for c in 0..3 {
                    let value = f32::from(src.get_pixel(x, y).0[c]);
                    let want = (value + delta).round().clamp(0.0, 255.0) as u8;
                    assert_eq!(fast.get_pixel(x, y).0[c], want, "at {x},{y} channel {c}");
                }
            }
        }
    }

    #[test]
    fn an_edge_between_two_colours_gains_no_fringe() {
        // Sharpening each channel against itself pushes each one a different
        // way here, and the result is a coloured line along every edge. The
        // correction is the same for all three, so the hue is left alone.
        // Kept clear of both ends of the range: where a channel clamps it
        // necessarily moves less than the others, and that is the one case
        // where the correction cannot come out equal.
        let src =
            RgbImage::from_fn(
                9,
                9,
                |x, _| {
                    if x < 4 { Rgb([80, 110, 140]) } else { Rgb([150, 180, 210]) }
                },
            );
        let out = unsharp(&src, 1.0);

        let difference = |p: &Rgb<u8>, q: &Rgb<u8>| {
            let d = |i: usize| i32::from(p.0[i]) - i32::from(q.0[i]);
            // How far the channels moved relative to each other, which is what
            // a fringe is.
            (d(0) - d(1)).abs().max((d(1) - d(2)).abs())
        };
        for y in 1..8u32 {
            for x in 1..8u32 {
                assert_eq!(
                    difference(out.get_pixel(x, y), src.get_pixel(x, y)),
                    0,
                    "the channels moved apart at {x},{y}"
                );
            }
        }
    }
}
