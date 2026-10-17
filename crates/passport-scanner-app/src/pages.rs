//! Deciding which document a capture belongs to.
//!
//! A card has two faces and the scanner sees one at a time, so a capture is
//! half a document. Something has to decide whether the capture that just
//! arrived is the second face of the card before it or the first face of a new
//! one, and nothing in the image answers that: two faces of one card have less
//! in common with each other than the fronts of two different cards do.
//!
//! What is left is the order they arrived in, which is also what the person at
//! the scanner is thinking: put it in, turn it over, put it in again. So a
//! document stays open for exactly one more capture, and anyone who wants only
//! one face says so by closing it.
//! Explicit side captures also retain the selected document after its pair is
//! complete, so either face can be retaken without creating a different document.
//!
//! Kept apart from the service that uses it, because this is the part worth
//! being sure about and it needs no scanner to check.

use passport_scanner_protocol::Side;

/// Where a capture belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    /// The document it is a face of.
    pub document: String,
    /// Which face.
    pub side: Side,
}

/// Groups captures into documents as they arrive.
#[derive(Debug, Clone, Default)]
pub struct Album {
    /// The document still waiting for its second face.
    open: Option<String>,
    current: Option<String>,
    explicit_side: Option<Side>,
}

impl Album {
    /// Nothing scanned yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The document waiting for a second face, if there is one.
    #[must_use]
    pub fn open_document(&self) -> Option<&str> {
        self.open.as_deref()
    }

    /// Place a capture, given an identifier to use if it starts a document.
    ///
    /// The identifier is passed in rather than generated here so that what
    /// makes an id — a clock, a counter, a random source — stays the caller's
    /// business and this stays testable.
    pub fn place(&mut self, new_document: impl Into<String>) -> Placement {
        let new_document = new_document.into();
        if let Some(side) = self.explicit_side.take() {
            let document = self.current.clone().unwrap_or(new_document);
            self.current = Some(document.clone());
            self.open = (side == Side::Front).then(|| document.clone());
            return Placement { document, side };
        }
        match self.open.take() {
            Some(document) => Placement { document, side: Side::Back },
            None => {
                let document = new_document;
                self.current = Some(document.clone());
                self.open = Some(document.clone());
                Placement { document, side: Side::Front }
            }
        }
    }

    /// Close the open document, so the next capture starts a new one.
    ///
    /// Does nothing when none is open, which makes it safe to call whenever a
    /// front end feels like it.
    pub fn finish(&mut self) {
        self.open = None;
        self.current = None;
        self.explicit_side = None;
    }

    /// Select an existing document for an explicit side capture or retake.
    pub fn select(&mut self, document: String) {
        self.current = Some(document);
    }

    /// Choose the side of the next capture; automatic placement still alternates.
    pub fn set_side(&mut self, side: Side) {
        self.explicit_side = Some(side);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_retake_replaces_the_selected_side_without_starting_another_document() {
        let mut album = Album::new();
        album.place("a");
        album.place("unused");
        album.place("b");
        album.select("a".into());
        album.set_side(Side::Back);
        assert_eq!(album.place("unused"), Placement { document: "a".into(), side: Side::Back });
        album.set_side(Side::Front);
        assert_eq!(album.place("unused").document, "a");
        album.finish();
        album.set_side(Side::Back);
        assert_eq!(album.place("c"), Placement { document: "c".into(), side: Side::Back });
    }

    #[test]
    fn the_first_capture_opens_a_document_as_its_front() {
        let mut album = Album::new();
        let placed = album.place("d1");
        assert_eq!(placed, Placement { document: "d1".to_owned(), side: Side::Front });
        assert_eq!(album.open_document(), Some("d1"));
    }

    #[test]
    fn the_second_is_the_back_of_the_same_document() {
        let mut album = Album::new();
        album.place("d1");
        let placed = album.place("d2");
        assert_eq!(
            placed,
            Placement { document: "d1".to_owned(), side: Side::Back },
            "the identifier offered for a new document should go unused"
        );
    }

    #[test]
    fn a_third_capture_starts_a_new_document() {
        let mut album = Album::new();
        album.place("d1");
        album.place("unused");
        let placed = album.place("d2");
        assert_eq!(placed, Placement { document: "d2".to_owned(), side: Side::Front });
    }

    #[test]
    fn two_cards_scanned_front_and_back_come_out_as_two_documents() {
        let mut album = Album::new();
        let placements: Vec<_> =
            ["a", "b", "c", "d"].into_iter().map(|id| album.place(id)).collect();

        assert_eq!(placements[0].document, placements[1].document);
        assert_eq!(placements[2].document, placements[3].document);
        assert_ne!(placements[0].document, placements[2].document);
        assert_eq!(
            placements.iter().map(|p| p.side).collect::<Vec<_>>(),
            [Side::Front, Side::Back, Side::Front, Side::Back]
        );
    }

    #[test]
    fn finishing_keeps_the_next_capture_out_of_the_last_document() {
        // One face was all that was wanted. Without this the next card would
        // be filed as the back of it.
        let mut album = Album::new();
        album.place("d1");
        album.finish();
        assert_eq!(album.open_document(), None);
        assert_eq!(album.place("d2"), Placement { document: "d2".to_owned(), side: Side::Front });
    }

    #[test]
    fn finishing_a_complete_document_changes_nothing() {
        let mut album = Album::new();
        album.place("d1");
        album.place("unused");
        album.finish();
        assert_eq!(album.place("d2"), Placement { document: "d2".to_owned(), side: Side::Front });
    }

    #[test]
    fn finishing_with_nothing_open_is_harmless() {
        let mut album = Album::new();
        album.finish();
        album.finish();
        assert_eq!(album.place("d1").side, Side::Front);
    }
}
