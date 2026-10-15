//! What the `AskUserQuestion` tool needs to know about `/loop auto`. The tool
//! runs on its own thread, so the state is process-wide and atomic. The mode
//! sets it when a run starts and clears it when the run ends.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static HALT: Mutex<Option<String>> = Mutex::new(None);

fn halt_slot() -> std::sync::MutexGuard<'static, Option<String>> {
    HALT.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether a `/loop auto` run is in progress: no user is there to answer.
pub fn is_active() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}

/// Mark a run started or ended. Ending a run drops any halt not yet taken.
pub fn set_active(active: bool) {
    ACTIVE.store(active, Ordering::SeqCst);
    if !active {
        halt_slot().take();
    }
}

/// `LOOP_HALT`: the run should stop after the current turn, for `reason`.
pub fn halt(reason: &str) {
    *halt_slot() = Some(reason.to_string());
}

/// The reason of a halt requested since the last call, if any.
pub fn take_halt() -> Option<String> {
    halt_slot().take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_halt_is_taken_once_and_ending_the_run_drops_it() {
        set_active(true);
        assert!(is_active());
        halt("no recommended answer");
        assert_eq!(take_halt().as_deref(), Some("no recommended answer"));
        assert_eq!(take_halt(), None);
        halt("left over");
        set_active(false);
        assert!(!is_active());
        assert_eq!(take_halt(), None);
    }
}
