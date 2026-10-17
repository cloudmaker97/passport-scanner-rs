//! The small mark beside a document in the history that says how its MRZ read.
//!
//! Drawn rather than typed: the bundled fonts carry no check mark or cross,
//! and the browser client has no system fonts to fall back on.
use crate::model::{Document, Verification, verification};
use iced::widget::canvas::{self, Frame, Geometry, LineCap, LineJoin, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};

/// Side length of the mark, in logical pixels.
pub(crate) const SIZE: f32 = 12.0;

/// How a document's machine-readable zone came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mrz {
    /// Read, and every check digit verified.
    Verified,
    /// A zone was read, but its checks failed or could not be confirmed.
    Wrong,
    /// Nothing that looks like a zone on either side.
    Absent,
}

impl Mrz {
    pub(crate) fn of(document: &Document) -> Self {
        match document.recognised().map(verification) {
            Some(Verification::Verified) => Self::Verified,
            Some(Verification::Failed | Verification::Inconclusive) => Self::Wrong,
            None => Self::Absent,
        }
    }
}

/// A [`Mrz`] drawn in the interface's colours.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Badge {
    pub(crate) mrz: Mrz,
    pub(crate) good: Color,
    pub(crate) bad: Color,
    pub(crate) muted: Color,
}

impl<Message> canvas::Program<Message> for Badge {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let s = bounds.width.min(bounds.height);
        let at = |x: f32, y: f32| Point::new(x * s, y * s);
        // Bare strokes in the status colour, nothing around them.
        let pen = |color| {
            Stroke::default()
                .with_color(color)
                .with_width(s * 0.14)
                .with_line_cap(LineCap::Round)
                .with_line_join(LineJoin::Round)
        };
        let (path, color) = match self.mrz {
            Mrz::Verified => (
                Path::new(|p| {
                    p.move_to(at(0.15, 0.55));
                    p.line_to(at(0.4, 0.8));
                    p.line_to(at(0.85, 0.25));
                }),
                self.good,
            ),
            Mrz::Wrong => (
                Path::new(|p| {
                    p.move_to(at(0.22, 0.22));
                    p.line_to(at(0.78, 0.78));
                    p.move_to(at(0.78, 0.22));
                    p.line_to(at(0.22, 0.78));
                }),
                self.bad,
            ),
            Mrz::Absent => (Path::line(at(0.25, 0.5), at(0.75, 0.5)), self.muted),
        };
        frame.stroke(&path, pen(color));
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use passport_scanner_protocol::{Field, FieldKind, Scan, Side};

    fn document(mrz: bool, check: Option<bool>) -> Document {
        let scan = Scan {
            id: "s".into(),
            document: "d".into(),
            side: Side::Front,
            at: String::new(),
            format: None,
            mrz: if mrz { vec!["I<UTO".into()] } else { vec![] },
            mrz_check: check,
            fields: if mrz {
                vec![Field {
                    kind: FieldKind::DocumentNumber,
                    value: "D1".into(),
                    check,
                    span: None,
                }]
            } else {
                vec![]
            },
            images: vec![],
        };
        Document { id: "d".into(), front: Some(scan), back: None }
    }

    #[test]
    fn a_verified_zone_is_good_a_read_but_unconfirmed_one_is_not_and_none_is_neutral() {
        assert_eq!(Mrz::of(&document(true, Some(true))), Mrz::Verified);
        assert_eq!(Mrz::of(&document(true, Some(false))), Mrz::Wrong);
        assert_eq!(Mrz::of(&document(true, None)), Mrz::Wrong);
        assert_eq!(Mrz::of(&document(false, None)), Mrz::Absent);
    }
}
