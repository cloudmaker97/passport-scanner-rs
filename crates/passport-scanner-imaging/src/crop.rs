//! Finding the document on the platen and cutting the rest away.
//!
//! Most of a capture is not the document. The platen is larger than an ID-1
//! card by some margin, and what surrounds the card is a dark moulding with
//! ribs and screw heads in it - of no interest to a reader, and actively
//! unhelpful to everything downstream: it drags the tone measurements toward
//! the platen, it is most of what a viewer has to look past, and it is most of
//! the bytes.
//!
//! Pure geometry over a grey buffer, so the part that decides where the
//! document is stays testable without a camera.

use passport_scanner_core::Rect;

/// How far outside the found box to cut, as a fraction of the box.
///
/// Thresholding lands just inside the document, because the outermost pixels
/// of a card are a rounded, shaded edge rather than its face. A little margin
/// puts the edge back and makes it visible that the whole card was caught.
pub const MARGIN: f32 = 0.01;

/// Find the document's main band of pixels brighter than `threshold`.
///
/// The platen is dark and the document is not, so a plain threshold separates
/// them. What is measured is the longest *unbroken* bright run across a row or
/// column, not how many bright pixels it holds. That distinction is what makes
/// this survive a corrected frame: flat-field correction multiplies the dim
/// end of the platen by up to five, and with it the sensor noise there, so a
/// row of bare platen can easily come out a tenth bright - enough to pass a
/// count, nowhere near enough to make an unbroken run. A document gives a run
/// the width of itself. Detached bright reflections below that main band are
/// excluded, while gaps from print and dark portraits are retained.
///
/// Returns `None` when nothing bright enough is there, or when what is there
/// is too small to be a document.
#[must_use]
pub fn bounds(grey: &[u8], width: u32, height: u32, threshold: u8) -> Option<Rect> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || grey.len() < w * h {
        return None;
    }

    // A document spans a good part of whichever way you measure it.
    let min_run = (w / 6).max(8);
    let min_col = (h / 6).max(8);

    let longest = |n: usize, at: &dyn Fn(usize) -> u8| {
        let (mut best, mut run) = (0usize, 0usize);
        for i in 0..n {
            if at(i) >= threshold {
                run += 1;
                best = best.max(run);
            } else {
                run = 0;
            }
        }
        best
    };
    let bright_in_row = |y: usize| longest(w, &|x| grey[y * w + x]);
    // A second, detached patch can be a reflection of the ID lamp in the
    // platen. Taking the first and last bright rows includes both patches.
    // Keep the largest coherent band instead, allowing short gaps for print
    // and portrait shadows. Then measure columns only within that band so a
    // reflection elsewhere cannot stretch the document sideways either.
    let (mut top, mut bottom) = largest_band(h, (h / 12).max(1), |y| bright_in_row(y) >= min_run)?;
    let columns = |top: usize, bottom: usize| {
        let band_height = bottom - top + 1;
        let bright_in_col = |x: usize| longest(band_height, &|y| grey[(top + y) * w + x]);
        // Do not split the horizontal extent into bands: a dark portrait can
        // separate a card's left edge from its light text area by many columns.
        let left = (0..w).find(|&x| bright_in_col(x) >= min_col)?;
        let right = (0..w).rev().find(|&x| bright_in_col(x) >= min_col)?;
        Some((left, right))
    };
    let (left, right) = columns(top, bottom)?;

    // A card lying sideways is narrow on the sensor, and its dark portrait can
    // then span nearly its whole width: those rows keep only thin bright
    // margins and fail the run test above, cutting the portrait off. Rows that
    // are bright at both of the card's edges are still card, so grow the band
    // through them. A detached reflection is neither contiguous nor bright at
    // both edges, so it stays out.
    // The card's own light border, looked for near each side. Dark print can
    // come within a few dozen pixels of the edge, and a card lying a degree
    // askew or with rounded corners moves that edge inward along its length,
    // so the window is wide and a tenth of it being bright is enough. Bare
    // platen has next to no bright pixels.
    let edge = ((right - left + 1) / 20).max(3);
    let card_row = |y: usize| {
        let bright = |range: std::ops::Range<usize>| {
            range.clone().filter(|&x| grey[y * w + x] >= threshold).count() * 10 >= range.len()
        };
        bright(left..left + edge) && bright(right + 1 - edge..right + 1)
    };
    let gap = (h / 100).max(1);
    let grow = |mut at: usize, step: fn(usize) -> Option<usize>| {
        let mut missed = 0;
        let mut probe = at;
        while let Some(next) = step(probe).filter(|&y| y < h) {
            probe = next;
            if card_row(probe) {
                at = probe;
                missed = 0;
            } else {
                missed += 1;
                if missed > gap {
                    break;
                }
            }
        }
        at
    };
    top = grow(top, |y| y.checked_sub(1));
    bottom = grow(bottom, |y| y.checked_add(1));
    let (left, right) = columns(top, bottom)?;

    if bottom <= top || right <= left {
        return None;
    }
    Some(Rect {
        x: left as u32,
        y: top as u32,
        w: (right - left + 1) as u32,
        h: (bottom - top + 1) as u32,
    })
}

/// Inclusive extent of the band with most supported rows/columns. Empty gaps
/// are tolerated up to `max_gap`, but do not contribute to its support.
fn largest_band(
    n: usize,
    max_gap: usize,
    mut present: impl FnMut(usize) -> bool,
) -> Option<(usize, usize)> {
    let (mut start, mut last, mut support) = (0, 0, 0);
    let (mut best, mut best_support) = (None, 0);
    for i in 0..n {
        if !present(i) {
            continue;
        }
        if support == 0 || i - last > max_gap + 1 {
            start = i;
            support = 0;
        }
        last = i;
        support += 1;
        if support > best_support {
            best = Some((start, last));
            best_support = support;
        }
    }
    best
}

/// The box to actually cut to: the document, grown by [`MARGIN`], or the whole
/// image when no plausible document was found.
///
/// Falling back to the whole image rather than failing is deliberate. A scan
/// of something this cannot locate - a dark passport cover, a document lying
/// off the platen - is still a scan, and handing back an uncropped image is a
/// better answer than handing back nothing.
#[must_use]
pub fn region(grey: &[u8], width: u32, height: u32, threshold: u8) -> Rect {
    match bounds(grey, width, height, threshold) {
        Some(found) if found.plausible(width, height) => {
            let margin = (found.w.min(found.h) as f32 * MARGIN).round() as u32;
            found.grown(margin, width, height)
        }
        _ => Rect::whole(width, height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dark field with a bright rectangle in it, like a card on the platen.
    fn scene(w: u32, h: u32, card: Rect) -> Vec<u8> {
        let mut px = vec![10u8; (w * h) as usize];
        for y in card.y..card.y + card.h {
            for x in card.x..card.x + card.w {
                px[(y * w + x) as usize] = 230;
            }
        }
        px
    }

    #[test]
    fn finds_a_card_on_a_dark_platen() {
        let card = Rect { x: 60, y: 40, w: 300, h: 180 };
        assert_eq!(bounds(&scene(400, 300, card), 400, 300, 128), Some(card));
    }

    /// A card lying sideways: narrow on the sensor, its dark portrait spans
    /// nearly the whole card width, leaving only thin bright margins beside it.
    #[test]
    fn a_sideways_card_keeps_its_wide_dark_portrait() {
        let card = Rect { x: 140, y: 20, w: 180, h: 260 };
        let mut px = scene(400, 300, card);
        for y in 190..270 {
            for x in 160..300 {
                px[y * 400 + x] = 15;
            }
        }
        assert_eq!(bounds(&px, 400, 300, 128), Some(card));
        // As captured: the portrait reaches to within a few pixels of one edge.
        for y in 190..270 {
            for x in 300..314 {
                px[y * 400 + x] = 15;
            }
        }
        assert_eq!(bounds(&px, 400, 300, 128), Some(card));
    }

    #[test]
    fn detached_lamp_reflections_do_not_extend_the_document() {
        let card = Rect { x: 60, y: 20, w: 300, h: 170 };
        let mut px = scene(400, 300, card);
        // Both reflections satisfy the old row-run test, but are detached
        // from the card and too short to be its main body.
        for y in 245..270 {
            for x in (30..110).chain(270..360) {
                px[y * 400 + x] = 255;
            }
        }
        assert_eq!(bounds(&px, 400, 300, 128), Some(card));
    }

    #[test]
    fn a_dark_print_band_does_not_split_the_card() {
        let card = Rect { x: 60, y: 40, w: 300, h: 180 };
        let mut px = scene(400, 300, card);
        for y in 100..118 {
            for x in 60..360 {
                px[y * 400 + x] = 20;
            }
        }
        assert_eq!(bounds(&px, 400, 300, 128), Some(card));
    }

    #[test]
    fn dark_portrait_does_not_discard_the_left_edge() {
        let card = Rect { x: 60, y: 40, w: 300, h: 180 };
        let mut px = scene(400, 300, card);
        for y in 40..220 {
            for x in 85..175 {
                px[y * 400 + x] = 20;
            }
        }
        assert_eq!(bounds(&px, 400, 300, 128), Some(card));
    }

    #[test]
    fn a_single_bright_speck_does_not_become_the_document() {
        let mut px = vec![10u8; 400 * 300];
        px[150 * 400 + 200] = 255;
        assert_eq!(bounds(&px, 400, 300, 128), None);
    }

    /// Speckle a region, the way amplified sensor noise falls: scattered, not
    /// in rows or columns. A fixed sequence, so the test cannot pass by luck.
    fn speckle(px: &mut [u8], w: usize, region: Rect, every: u64) {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for y in region.y as usize..(region.y + region.h) as usize {
            for x in region.x as usize..(region.x + region.w) as usize {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                if (state >> 33) % every == 0 {
                    px[y * w + x] = 240;
                }
            }
        }
    }

    #[test]
    fn scattered_noise_is_not_a_document() {
        // What flat-field correction does to the dim end of the platen: it
        // multiplies the noise there by up to five. Roughly a seventh of the
        // pixels come out bright - enough to pass a count of them, nowhere
        // near enough to make an unbroken run.
        let mut px = vec![10u8; 400 * 300];
        speckle(&mut px, 400, Rect::whole(400, 300), 7);
        assert_eq!(bounds(&px, 400, 300, 128), None);
    }

    #[test]
    fn a_document_beside_that_noise_is_found_without_it() {
        let card = Rect { x: 220, y: 60, w: 150, h: 180 };
        let mut px = scene(400, 300, card);
        speckle(&mut px, 400, Rect { x: 0, y: 0, w: 200, h: 300 }, 7);

        let found = bounds(&px, 400, 300, 128).expect("the card is still there");
        assert!(found.x >= 200, "the noise dragged the box out to {}", found.x);
        assert!(found.w <= card.w + 8, "and stretched it to {} wide", found.w);
    }

    #[test]
    fn a_rib_of_glare_down_the_moulding_does_not_either() {
        // One bright column the full height of the frame: enough to pass the
        // column test on its own, but it is one column, so the box it would
        // give has no width and is rejected.
        let mut px = vec![10u8; 400 * 300];
        for y in 0..300 {
            px[y * 400 + 7] = 255;
        }
        assert_eq!(bounds(&px, 400, 300, 128), None);
    }

    #[test]
    fn an_empty_platen_finds_nothing() {
        assert_eq!(bounds(&vec![10u8; 400 * 300], 400, 300, 128), None);
    }

    #[test]
    fn a_buffer_shorter_than_its_size_is_refused_rather_than_read_past() {
        assert_eq!(bounds(&[200u8; 10], 400, 300, 128), None);
        assert_eq!(bounds(&[], 0, 0, 128), None);
    }

    #[test]
    fn the_region_is_the_card_with_a_little_room_around_it() {
        let card = Rect { x: 60, y: 40, w: 300, h: 180 };
        let r = region(&scene(400, 300, card), 400, 300, 128);
        assert!(r.x < card.x && r.y < card.y, "the box should sit outside the card");
        assert!(r.x + r.w > card.x + card.w && r.y + r.h > card.y + card.h);
        assert!(r.w < 400 && r.h < 300, "but still be a crop, not the whole frame");
    }

    #[test]
    fn nothing_findable_falls_back_to_the_whole_image() {
        assert_eq!(region(&vec![10u8; 400 * 300], 400, 300, 128), Rect::whole(400, 300));
    }

    #[test]
    fn something_too_small_to_be_a_document_falls_back_too() {
        // A found box, but card-sized only in one direction: a reflection off
        // a rib rather than a document.
        let sliver = Rect { x: 100, y: 100, w: 40, h: 150 };
        assert_eq!(region(&scene(400, 300, sliver), 400, 300, 128), Rect::whole(400, 300));
    }
}
