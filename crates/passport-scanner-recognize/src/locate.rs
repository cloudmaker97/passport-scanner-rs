//! Finding the zone on a document.
//!
//! Where the document itself is, is a question about the image rather than
//! about text, and is answered by [`passport_scanner_imaging::crop`]. What is left
//! here is the part that only matters once a document has been found: which
//! part of it carries the machine-readable zone.

use passport_scanner_core::Rect;

/// The strip of a document where a machine-readable zone sits.
///
/// Every ICAO layout puts the zone across the bottom. Taking the lower third
/// with a little margin catches all three formats without having to know which
/// one is in front of the camera.
#[must_use]
pub fn zone_strip(document: Rect) -> Rect {
    let h = (document.h / 3).max(1);
    let margin = document.w / 40;
    Rect {
        x: document.x + margin.min(document.w / 2),
        y: document.y + document.h.saturating_sub(h),
        w: document.w.saturating_sub(margin * 2).max(1),
        h,
    }
}

/// An OCR retry needs the edge characters and room above sloping MRZ rows.
/// Keep the full width and allow up to five degrees of vertical drift, capped
/// at half the document so a portrait or headline does not dominate deskew.
#[must_use]
pub fn zone_strip_for_skew(document: Rect) -> Rect {
    let drift = (f64::from(document.w) * 5_f64.to_radians().tan()).ceil() as u32;
    let h = (document.h / 3).saturating_add(drift).min(document.h.div_ceil(2)).max(1);
    Rect { y: document.y + document.h.saturating_sub(h), h, ..document }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zone_strip_sits_across_the_bottom() {
        let doc = Rect { x: 100, y: 100, w: 600, h: 300 };
        let strip = zone_strip(doc);
        assert_eq!(strip.y + strip.h, doc.y + doc.h, "the strip ends at the document's edge");
        assert!(strip.h <= doc.h / 2, "the strip is a band, not half the card");
        assert!(strip.x >= doc.x && strip.x + strip.w <= doc.x + doc.w, "it stays inside");
    }

    #[test]
    fn skew_retry_keeps_edge_characters_and_vertical_room_without_leaving_document() {
        let doc = Rect { x: 30, y: 20, w: 1800, h: 1100 };
        let normal = zone_strip(doc);
        let retry = zone_strip_for_skew(doc);
        assert_eq!((retry.x, retry.w), (doc.x, doc.w));
        assert_eq!(retry.y + retry.h, doc.y + doc.h);
        assert!(retry.y < normal.y);
        assert!(retry.h <= doc.h.div_ceil(2));
        let tiny = zone_strip_for_skew(Rect { x: 0, y: 0, w: 1, h: 1 });
        assert_eq!((tiny.w, tiny.h), (1, 1));
    }
}
