//! Document grouping without scanner or window dependencies.

use passport_scanner_protocol::{Scan, Side, Source};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Verification {
    Failed,
    Inconclusive,
    Verified,
}

/// Check digits validate the extracted MRZ, not document authenticity.
pub(crate) fn verification(scan: &Scan) -> Verification {
    if scan.mrz_check == Some(false) || scan.fields.iter().any(|field| field.check == Some(false)) {
        return Verification::Failed;
    }
    if scan.checks_passed() { Verification::Verified } else { Verification::Inconclusive }
}

#[derive(Debug, Clone)]
pub(crate) struct Document {
    pub(crate) id: String,
    pub(crate) front: Option<Scan>,
    pub(crate) back: Option<Scan>,
}

impl Document {
    pub(crate) fn page(&self, side: Side) -> Option<&Scan> {
        match side {
            Side::Front => self.front.as_ref(),
            Side::Back => self.back.as_ref(),
            _ => None,
        }
    }

    /// Prefer verified data, with the back as a tie-breaker. Never merge
    /// contradictory front/back fields or claim absent MRZ data was verified.
    pub(crate) fn recognised(&self) -> Option<&Scan> {
        [self.front.as_ref(), self.back.as_ref()]
            .into_iter()
            .flatten()
            .filter(|s| !s.fields.is_empty() || !s.mrz.is_empty())
            .max_by_key(|scan| (verification(scan), scan.side == Side::Back))
    }

    pub(crate) fn image(
        &self,
        side: Side,
        source: Source,
    ) -> Option<&passport_scanner_protocol::Image> {
        self.page(side)?.images.iter().find(|image| image.source == source)
    }

    pub(crate) fn image_count(&self) -> usize {
        ordered_slots().iter().filter(|(side, source)| self.image(*side, *source).is_some()).count()
    }

    pub(crate) fn representative_scan(&self) -> Option<&Scan> {
        self.front.as_ref().or(self.back.as_ref())
    }
}

/// The same visual order that the PDF exporter is required to use.
pub(crate) fn ordered_slots() -> [(Side, Source); 4] {
    [
        (Side::Front, Source::White),
        (Side::Back, Source::White),
        (Side::Front, Source::Infrared),
        (Side::Back, Source::Infrared),
    ]
}

#[derive(Debug, Default)]
pub(crate) struct Documents(pub(crate) Vec<Document>);

impl Documents {
    pub(crate) fn upsert(&mut self, scan: Scan) {
        if !self.0.iter().any(|d| d.id == scan.document) {
            self.0.insert(0, Document { id: scan.document.clone(), front: None, back: None });
        }
        let document =
            self.0.iter_mut().find(|d| d.id == scan.document).expect("inserted document");
        match scan.side {
            Side::Front => document.front = Some(scan),
            Side::Back => document.back = Some(scan),
            _ => {}
        }
    }

    pub(crate) fn get(&self, id: &str) -> Option<&Document> {
        self.0.iter().find(|d| d.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use passport_scanner_protocol::{Field, FieldKind};

    fn scan(id: &str, document: &str, side: Side) -> Scan {
        Scan {
            id: id.into(),
            document: document.into(),
            side,
            at: String::new(),
            format: None,
            mrz: vec![],
            mrz_check: None,
            fields: vec![],
            images: vec![],
        }
    }

    #[test]
    fn pairs_documents_and_replaces_only_the_named_face() {
        let mut docs = Documents::default();
        docs.upsert(scan("a", "one", Side::Front));
        docs.upsert(scan("b", "one", Side::Back));
        docs.upsert(scan("c", "two", Side::Front));
        docs.upsert(scan("d", "one", Side::Front));
        assert_eq!(docs.0.len(), 2);
        let one = docs.get("one").unwrap();
        assert_eq!(one.front.as_ref().unwrap().id, "d");
        assert_eq!(one.back.as_ref().unwrap().id, "b");
        assert_eq!(docs.0[0].id, "two");
    }

    #[test]
    fn empty_front_does_not_claim_recognition_and_back_is_authoritative() {
        let mut docs = Documents::default();
        docs.upsert(scan("a", "one", Side::Front));
        assert!(docs.get("one").unwrap().recognised().is_none());
        let mut back = scan("b", "one", Side::Back);
        back.fields.push(Field {
            kind: FieldKind::Surname,
            value: "SPECIMEN".into(),
            check: None,
            span: None,
        });
        docs.upsert(back);
        assert_eq!(docs.get("one").unwrap().recognised().unwrap().side, Side::Back);
    }

    fn recognised_scan(side: Side, composite: Option<bool>) -> Scan {
        let mut value = scan("recognised", "one", side);
        value.mrz = vec!["SPECIMEN".into()];
        value.mrz_check = composite;
        value.fields.push(Field {
            kind: FieldKind::DocumentNumber,
            value: "SPECIMEN".into(),
            check: Some(true),
            span: None,
        });
        value
    }

    #[test]
    fn composite_failure_overrides_passing_individual_digits() {
        assert_eq!(verification(&recognised_scan(Side::Back, Some(false))), Verification::Failed);
        assert_eq!(verification(&recognised_scan(Side::Back, None)), Verification::Inconclusive);
        assert_eq!(verification(&recognised_scan(Side::Back, Some(true))), Verification::Verified);
        let mut missing_fields = recognised_scan(Side::Back, Some(true));
        missing_fields.fields.clear();
        assert_eq!(verification(&missing_fields), Verification::Inconclusive);
    }

    #[test]
    fn verified_data_wins_and_back_breaks_equal_quality_ties() {
        let mut docs = Documents::default();
        docs.upsert(recognised_scan(Side::Front, Some(true)));
        docs.upsert(recognised_scan(Side::Back, Some(false)));
        assert_eq!(docs.get("one").unwrap().recognised().unwrap().side, Side::Front);
        docs.upsert(recognised_scan(Side::Back, Some(true)));
        assert_eq!(docs.get("one").unwrap().recognised().unwrap().side, Side::Back);
    }

    #[test]
    fn four_slots_are_white_front_white_back_ir_front_ir_back() {
        assert_eq!(
            ordered_slots(),
            [
                (Side::Front, Source::White),
                (Side::Back, Source::White),
                (Side::Front, Source::Infrared),
                (Side::Back, Source::Infrared)
            ]
        );
    }

    #[test]
    fn pdf_requires_both_illuminations_on_both_sides() {
        let mut docs = Documents::default();
        let mut front = scan("a", "one", Side::Front);
        front.images = [Source::White, Source::Infrared]
            .into_iter()
            .map(|source| passport_scanner_protocol::Image {
                source,
                url: "data:image/jpeg;base64,AA==".into(),
                width: 1,
                height: 1,
            })
            .collect();
        docs.upsert(front.clone());
        assert_eq!(docs.get("one").unwrap().image_count(), 2);
        let mut back = front;
        back.id = "b".into();
        back.side = Side::Back;
        docs.upsert(back.clone());
        assert_eq!(docs.get("one").unwrap().image_count(), 4);
        back.images.pop();
        docs.upsert(back);
        assert_eq!(docs.get("one").unwrap().image_count(), 3);
    }
}
