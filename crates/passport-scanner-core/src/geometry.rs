//! Rectangles in an image.
//!
//! Shared vocabulary: both the part that finds the document and the part that
//! reads text off it talk about the same boxes.

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: u32,
    /// Top edge.
    pub y: u32,
    /// Width.
    pub w: u32,
    /// Height.
    pub h: u32,
}

impl Rect {
    /// The whole of an image of this size.
    #[must_use]
    pub const fn whole(width: u32, height: u32) -> Self {
        Self { x: 0, y: 0, w: width, h: height }
    }

    /// Is this rectangle big enough to be a document rather than a reflection?
    #[must_use]
    pub const fn plausible(&self, image_w: u32, image_h: u32) -> bool {
        self.w > image_w / 5 && self.h > image_h / 8
    }

    /// Grow by `margin` pixels on every side, stopping at the image edges.
    ///
    /// A box found by thresholding sits just inside the document, because the
    /// outermost pixels of a card are a rounded, shaded edge. A margin puts
    /// them back.
    #[must_use]
    pub const fn grown(&self, margin: u32, width: u32, height: u32) -> Self {
        let x = self.x.saturating_sub(margin);
        let y = self.y.saturating_sub(margin);
        let right = if self.x + self.w + margin > width { width } else { self.x + self.w + margin };
        let bottom =
            if self.y + self.h + margin > height { height } else { self.y + self.h + margin };
        Self { x, y, w: right - x, h: bottom - y }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_whole_image_is_a_rectangle_like_any_other() {
        assert_eq!(Rect::whole(640, 480), Rect { x: 0, y: 0, w: 640, h: 480 });
    }

    #[test]
    fn a_card_sized_box_is_plausible_and_a_glint_is_not() {
        assert!(Rect { x: 10, y: 10, w: 300, h: 200 }.plausible(640, 480));
        assert!(!Rect { x: 10, y: 10, w: 40, h: 200 }.plausible(640, 480));
        assert!(!Rect { x: 10, y: 10, w: 300, h: 20 }.plausible(640, 480));
    }

    #[test]
    fn growing_stops_at_the_edges_rather_than_running_past_them() {
        let r = Rect { x: 5, y: 5, w: 100, h: 50 };
        assert_eq!(r.grown(10, 640, 480), Rect { x: 0, y: 0, w: 115, h: 65 });

        let edge = Rect { x: 600, y: 440, w: 40, h: 40 };
        assert_eq!(edge.grown(20, 640, 480), Rect { x: 580, y: 420, w: 60, h: 60 });
    }

    #[test]
    fn growing_by_nothing_changes_nothing() {
        let r = Rect { x: 7, y: 9, w: 11, h: 13 };
        assert_eq!(r.grown(0, 640, 480), r);
    }
}
