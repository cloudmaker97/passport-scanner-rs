//! Deciding when something put on the platen is ready to be scanned.
//!
//! The two sensors say what they see the instant they are asked, and a card
//! being pushed in crosses the first one well before it comes to rest. Acting
//! on the first reading scans a card that is still moving; acting on every
//! reading scans it again on the next poll. So the signal has to be held to
//! two conditions:
//!
//! - it has settled - the same reading twice running, which at a two-hundred
//!   millisecond poll is a fifth of a second of stillness;
//! - and it is new - nothing is scanned again until the platen has been seen
//!   empty, which is what separates one document from the next.
//!
//! Kept apart from the service that uses it, because a state machine that
//! decides when to fire at hardware is worth being certain about, and this
//! needs no hardware to check.

use passport_scanner_core::DocumentPresence;

/// How many identical readings in a row count as settled.
pub const STABLE: u8 = 2;

/// What the service should do about a reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Keep waiting.
    Wait,
    /// A document has settled on the platen and has not been scanned yet.
    Scan,
}

/// Turns a stream of sensor readings into one decision per placement.
#[derive(Debug, Clone, Default)]
pub struct Trigger {
    /// The last reading, and how many times running it has been seen.
    last: Option<(DocumentPresence, u8)>,
    /// Whether what is on the platen now has already been scanned.
    scanned: bool,
}

impl Trigger {
    /// Nothing seen yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Offer a reading, and find out whether to scan.
    pub fn observe(&mut self, presence: DocumentPresence) -> Action {
        self.observe_deferred(presence);
        if presence.is_present()
            && self.last.is_some_and(|(_, runs)| runs >= STABLE)
            && !self.scanned
        {
            self.scanned = true;
            return Action::Scan;
        }
        Action::Wait
    }

    /// Remember movement while processing a capture without consuming the next scan.
    /// A settled replacement remains eligible when normal observation resumes.
    pub fn observe_deferred(&mut self, presence: DocumentPresence) {
        let runs = match self.last {
            Some((previous, runs)) if previous == presence => runs.saturating_add(1),
            _ => 1,
        };
        self.last = Some((presence, runs));

        if !presence.is_present() {
            // The platen is clear, so whatever comes next is a new document.
            self.scanned = false;
        }
    }

    /// Treat what is on the platen as already dealt with.
    ///
    /// For a scan started by hand: without this the trigger would fire on the
    /// same document a moment later.
    pub fn mark_scanned(&mut self) {
        self.scanned = true;
    }

    /// Whether anything is on the platen, as last seen.
    #[must_use]
    pub fn present(&self) -> bool {
        self.last.is_some_and(|(p, _)| p.is_present())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DocumentPresence::{Absent, Full, Partial};

    /// Feed a run of readings and collect what was decided.
    fn run(readings: &[DocumentPresence]) -> Vec<Action> {
        let mut trigger = Trigger::new();
        readings.iter().map(|p| trigger.observe(*p)).collect()
    }

    #[test]
    fn a_card_that_has_come_to_rest_is_scanned_once() {
        let actions = run(&[Absent, Partial, Partial, Partial, Partial]);
        assert_eq!(actions, [Action::Wait, Action::Wait, Action::Scan, Action::Wait, Action::Wait]);
    }

    #[test]
    fn a_card_still_being_pushed_in_is_not() {
        // The first sensor trips, then the second: the reading changed, so it
        // has not settled, and the scan waits for the reading that repeats.
        let actions = run(&[Absent, Partial, Full, Full]);
        assert_eq!(actions, [Action::Wait, Action::Wait, Action::Wait, Action::Scan]);
    }

    #[test]
    fn leaving_it_on_the_platen_does_not_scan_it_again() {
        let actions = run(&[Partial; 12]);
        assert_eq!(actions.iter().filter(|a| **a == Action::Scan).count(), 1);
    }

    #[test]
    fn taking_it_out_and_putting_it_back_scans_again() {
        let actions = run(&[Partial, Partial, Absent, Partial, Partial]);
        assert_eq!(actions.iter().filter(|a| **a == Action::Scan).count(), 2);
    }

    #[test]
    fn turning_a_card_over_counts_as_a_new_placement() {
        // Out, over, in: exactly the sequence for a second face.
        let actions = run(&[Full, Full, Absent, Absent, Full, Full]);
        assert_eq!(actions.iter().filter(|a| **a == Action::Scan).count(), 2);
    }

    #[test]
    fn an_empty_platen_never_fires_however_long_it_is_watched() {
        assert!(run(&[Absent; 20]).iter().all(|a| *a == Action::Wait));
    }

    #[test]
    fn a_scan_started_by_hand_stops_the_trigger_repeating_it() {
        let mut trigger = Trigger::new();
        assert_eq!(trigger.observe(Partial), Action::Wait);
        trigger.mark_scanned();
        for _ in 0..5 {
            assert_eq!(trigger.observe(Partial), Action::Wait);
        }
        // Until it is taken off and put back.
        trigger.observe(Absent);
        trigger.observe(Partial);
        assert_eq!(trigger.observe(Partial), Action::Scan);
    }

    #[test]
    fn presence_reports_the_last_reading() {
        let mut trigger = Trigger::new();
        assert!(!trigger.present(), "nothing has been seen yet");
        trigger.observe(Partial);
        assert!(trigger.present());
        trigger.observe(Absent);
        assert!(!trigger.present());
    }

    #[test]
    fn replacement_during_processing_is_not_consumed() {
        let mut trigger = Trigger::new();
        trigger.observe(Full);
        assert_eq!(trigger.observe(Full), Action::Scan);
        for presence in [Absent, Partial, Full, Full, Full] {
            trigger.observe_deferred(presence);
        }
        assert_eq!(trigger.observe(Full), Action::Scan);
        assert_eq!(trigger.observe(Full), Action::Wait);
    }

    #[test]
    fn stationary_card_during_processing_is_not_repeated() {
        let mut trigger = Trigger::new();
        trigger.mark_scanned();
        for _ in 0..20 {
            trigger.observe_deferred(Full);
        }
        assert_eq!(trigger.observe(Full), Action::Wait);
    }

    #[test]
    fn replacement_removed_before_processing_finishes_is_not_scanned() {
        let mut trigger = Trigger::new();
        trigger.mark_scanned();
        for presence in [Absent, Full, Full, Absent] {
            trigger.observe_deferred(presence);
        }
        assert_eq!(trigger.observe(Absent), Action::Wait);
        assert_eq!(trigger.observe(Full), Action::Wait);
        assert_eq!(trigger.observe(Full), Action::Scan);
    }
}
