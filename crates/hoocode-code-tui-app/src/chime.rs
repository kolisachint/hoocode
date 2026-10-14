//! The completion chime (extras.md, item 2): one terminal bell when a turn that ran
//! longer than ten seconds finishes (not after an abort), or as soon as the agent asks
//! the user a question. Chimes within five seconds of the last one are dropped. The
//! caller checks the `chimeOnTurnComplete` setting first.

use std::time::{Duration, Instant};

use hoocode_ai_types::StopReason;

/// The bell character written to the terminal.
pub const BELL: &str = "\x07";
/// A turn must run for longer than this to chime when it finishes.
pub const TURN_CHIME_MIN: Duration = Duration::from_secs(10);
/// Chimes closer together than this are dropped.
pub const CHIME_DEBOUNCE: Duration = Duration::from_secs(5);

/// Whether a turn that ran for `elapsed` and ended with `stop_reason` chimes.
/// An abort never chimes, and neither does a turn with no final assistant message.
pub fn turn_chime_due(elapsed: Duration, stop_reason: Option<StopReason>) -> bool {
    elapsed > TURN_CHIME_MIN && matches!(stop_reason, Some(reason) if reason != StopReason::Aborted)
}

/// Drops the chimes that come within [`CHIME_DEBOUNCE`] of the last one rung.
#[derive(Debug, Default)]
pub struct ChimeDebounce {
    last: Option<Instant>,
}

impl ChimeDebounce {
    /// Whether a chime may ring at `now`, recording it when it may. A dropped chime
    /// does not move the window.
    pub fn allow(&mut self, now: Instant) -> bool {
        if let Some(last) = self.last {
            if now.saturating_duration_since(last) < CHIME_DEBOUNCE {
                return false;
            }
        }
        self.last = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_must_run_longer_than_ten_seconds() {
        let stop = Some(StopReason::Stop);
        assert!(!turn_chime_due(Duration::from_secs(3), stop));
        assert!(
            !turn_chime_due(Duration::from_secs(10), stop),
            "exactly 10 s is not longer"
        );
        assert!(turn_chime_due(Duration::from_millis(10_001), stop));
    }

    #[test]
    fn aborted_or_unfinished_turns_do_not_chime() {
        let long = Duration::from_secs(60);
        assert!(!turn_chime_due(long, Some(StopReason::Aborted)));
        assert!(!turn_chime_due(long, None));
        assert!(turn_chime_due(long, Some(StopReason::Stop)));
        assert!(
            turn_chime_due(long, Some(StopReason::Error)),
            "a failed turn still finished"
        );
    }

    #[test]
    fn repeats_within_five_seconds_are_dropped() {
        let start = Instant::now();
        let mut debounce = ChimeDebounce::default();
        assert!(debounce.allow(start));
        assert!(!debounce.allow(start + Duration::from_secs(1)));
        assert!(!debounce.allow(start + Duration::from_millis(4_900)));
        assert!(
            debounce.allow(start + CHIME_DEBOUNCE),
            "five seconds later rings again"
        );
    }

    #[test]
    fn a_dropped_chime_does_not_move_the_window() {
        let start = Instant::now();
        let mut debounce = ChimeDebounce::default();
        assert!(debounce.allow(start));
        assert!(!debounce.allow(start + Duration::from_secs(3)));
        assert!(debounce.allow(start + Duration::from_secs(6)));
        assert!(!debounce.allow(start + Duration::from_secs(8)));
    }
}
