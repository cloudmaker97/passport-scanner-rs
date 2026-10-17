//! The small mark beside a document in the history that says how its MRZ read.
//!
//! Drawn rather than typed: the bundled fonts carry no check mark or cross,
//! and the browser client has no system fonts to fall back on.
use crate::model::{Document, Verification, verification};
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Theme, mouse};

/// Side length of the mark, in logical pixels.
pub(crate) const SIZE: f32 = 14.0;

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
        let centre = Point::new(s / 2.0, s / 2.0);
        let at = |x: f32, y: f32| Point::new(x * s, y * s);
        let mark = |color| Stroke::default().with_color(color).with_width(s * 0.13);
        match self.mrz {
            Mrz::Verified => {
                frame.fill(&Path::circle(centre, s / 2.0), self.good);
                let tick = Path::new(|p| {
                    p.move_to(at(0.28, 0.52));
                    p.line_to(at(0.44, 0.68));
                    p.line_to(at(0.73, 0.36));
                });
                frame.stroke(&tick, mark(Color::WHITE));
            }
            Mrz::Wrong => {
                frame.fill(&Path::circle(centre, s / 2.0), self.bad);
                let cross = Path::new(|p| {
                    p.move_to(at(0.33, 0.33));
                    p.line_to(at(0.67, 0.67));
                    p.move_to(at(0.67, 0.33));
                    p.line_to(at(0.33, 0.67));
                });
                frame.stroke(&cross, mark(Color::WHITE));
            }
            Mrz::Absent => {
                let ring = Stroke::default().with_color(self.muted).with_width(s * 0.09);
                frame.stroke(&Path::circle(centre, s / 2.0 - s * 0.06), ring);
                frame.stroke(&Path::line(at(0.32, 0.5), at(0.68, 0.5)), mark(self.muted));
            }
        }
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
