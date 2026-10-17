//! Telling a fresh frame from one the stream was still holding.
//!
//! After a lamp change the capture stack keeps handing back frames that were
//! exposed under the previous lamp. The earlier answer was to throw away a
//! fixed number of them, which is a guess: measured over nine scans, two was
//! sometimes too few and four was sometimes no better, because how far behind
//! the stream runs is not constant.
//!
//! So ask the frame instead of counting. A JPEG of a card under white light
//! and the same card under infrared are not the same size — on the reference
//! unit they differ by about a quarter — and consecutive frames of the same
//! steady picture are within a per cent or two of each other. That is enough
//! to recognise the change without decoding anything.
//!
//! Size is not enough on its own, though, and the reason is timing: a stream
//! that has been running hands back buffered frames instantly, and two
//! buffered frames of the same stale picture are as steady as two fresh ones.
//! So how long a frame took to arrive is part of the test — see
//! [`was_buffered`].

use std::time::Duration;

/// How far two sizes may differ and still be the same picture.
pub const STEADY: f64 = 0.02;

/// How far a frame must differ from the one before the change to be new.
///
/// Well below the roughly one-in-four the two lamps actually produce, and well
/// above the one-in-fifty that consecutive frames vary by.
pub const CHANGED: f64 = 0.05;

// If a steady pair could also read as unchanged, the loop below would never
// settle on anything. Checked here rather than in a test, so it cannot be got
// wrong in the first place.
const _: () = assert!(STEADY < CHANGED);

/// Below this, a frame was already waiting rather than just taken.
///
/// A five-megapixel frame takes about a quarter of a second to come over the
/// wire, so one that arrives in a fraction of that was sitting in the stream's
/// buffer — which means it was exposed before whatever this scan just changed.
/// Eighty milliseconds is well under a real frame interval and far above the
/// no time at all a buffered one takes.
///
/// This is what separates the first scan after opening a device, which was
/// always right, from every scan after it, which mostly were not: a cold
/// stream has nothing buffered and a warm one has two or three frames of the
/// previous exposure ready to hand back instantly.
pub const STALE_UNDER: Duration = Duration::from_millis(80);

/// Whether a frame arrived too quickly to have been taken just now.
#[must_use]
pub fn was_buffered(elapsed: Duration) -> bool {
    elapsed < STALE_UNDER
}

/// How many frames to pull at most while clearing the stream's buffer.
///
/// A ceiling on [`was_buffered`], because a stream that keeps handing frames
/// back instantly would otherwise be waited on for ever. Deep enough for any
/// buffer this stack has been seen to hold.
pub const DRAIN: usize = 8;

// A drain of zero would take no frame at all, and an unbounded one would hang
// a scan on a stream that never slows down. Checked here rather than in a test
// so it cannot be got wrong in the first place.
const _: () = assert!(DRAIN > 0 && DRAIN <= 16);

/// How many frames to take at most while waiting.
///
/// A ceiling, not a target: if the stream has not caught up by here, the last
/// frame is used anyway. A slightly stale image is a better outcome than a
/// scan that fails.
pub const LIMIT: usize = 5;

/// Relative difference between two sizes, as a fraction of the larger.
#[must_use]
pub fn difference(a: usize, b: usize) -> f64 {
    let larger = a.max(b);
    if larger == 0 {
        return 0.0;
    }
    a.abs_diff(b) as f64 / larger as f64
}

/// Whether a frame is the new picture, settled.
///
/// - `size` is the frame just taken.
/// - `previous` is the one before it, if any; without it nothing can be said
///   about whether the picture has stopped moving.
/// - `before` is the last frame from before the change. Without one — the
///   first capture of a run — there is nothing to have changed from, so only
///   steadiness is required.
#[must_use]
pub fn is_settled(size: usize, previous: Option<usize>, before: Option<usize>) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if difference(size, previous) > STEADY {
        return false;
    }
    before.is_none_or(|before| difference(size, before) >= CHANGED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_that_arrived_instantly_was_waiting_in_the_buffer() {
        assert!(was_buffered(Duration::from_millis(0)));
        assert!(was_buffered(Duration::from_millis(12)));
    }

    #[test]
    fn a_frame_that_took_a_frame_interval_is_a_fresh_one() {
        // Around 240 ms on the reference unit at five megapixels.
        assert!(!was_buffered(Duration::from_millis(240)));
        assert!(!was_buffered(Duration::from_millis(100)));
    }

    /// Sizes taken from a real pair of captures on the reference unit.
    const WHITE: usize = 1_661_992;
    const INFRARED: usize = 2_116_930;

    #[test]
    fn two_frames_of_the_same_new_picture_are_settled() {
        assert!(is_settled(INFRARED, Some(INFRARED - 9_000), Some(WHITE)));
    }

    #[test]
    fn a_frame_still_showing_the_previous_lamp_is_not() {
        // The stream is behind: both frames are the old picture.
        assert!(!is_settled(WHITE, Some(WHITE - 4_000), Some(WHITE)));
    }

    #[test]
    fn a_frame_caught_mid_change_is_not() {
        // It has changed, but the one before it was something else again, so
        // the picture is still moving.
        assert!(!is_settled(INFRARED, Some(WHITE), Some(WHITE)));
    }

    #[test]
    fn the_first_frame_of_any_run_is_never_settled() {
        // Nothing to compare against yet.
        assert!(!is_settled(WHITE, None, None));
        assert!(!is_settled(WHITE, None, Some(INFRARED)));
    }

    #[test]
    fn with_nothing_before_it_steadiness_is_enough() {
        // The first source of a scan: there is no previous lamp to differ from.
        assert!(is_settled(WHITE, Some(WHITE + 5_000), None));
        assert!(!is_settled(WHITE, Some(WHITE / 2), None));
    }

    #[test]
    fn the_difference_is_symmetric_and_bounded() {
        assert!((difference(100, 100) - 0.0).abs() < f64::EPSILON);
        assert!((difference(50, 100) - 0.5).abs() < f64::EPSILON);
        assert!((difference(100, 50) - 0.5).abs() < f64::EPSILON);
        assert!((difference(0, 0) - 0.0).abs() < f64::EPSILON);
        assert!((difference(0, 10) - 1.0).abs() < f64::EPSILON);
    }
}
