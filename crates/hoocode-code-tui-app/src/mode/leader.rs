//! The leader key at the prompt: Ctrl+X, then one key (`docs/design/themes-keybindings.md`, Part K).
//!
//! The leader sees a key before the screen does. A key it takes (the leader,
//! a key typed after it, or the Esc that cancels it) never reaches the editor
//! or the scroll view, so it cannot interrupt a turn. The footer shows the
//! pending state, and the which-key popup lists the leader keys after
//! [`WHICH_KEY_DELAY`]. Keys come from the keymap, so a user's changes show.

use std::rc::Rc;
use std::time::{Duration, Instant};

use hoocode_code_tui_keybindings::{
    leader_label, AppKeybindingsManager, Context, LeaderEntry, WHICH_KEY_DELAY,
};
use hoocode_tui_keys::{is_key_release, CancelReason, LeaderMachine, LeaderOutcome};

use super::input::action_for_binding;
use super::*;
use crate::transient_lines::LeaderLine;

/// How long a note such as "no binding" stays in the footer.
const NOTE_TIME: Duration = Duration::from_millis(1500);

/// The prompt's leader: the machine, the popup's entries, and the footer note.
pub(super) struct LeaderState {
    machine: LeaderMachine,
    entries: Vec<LeaderEntry>,
    /// The leader as the footer shows it (`Ctrl+X`).
    label: String,
    /// A note after a cancel that ran nothing, and when it expires.
    note: Option<(&'static str, Instant)>,
}

impl LeaderState {
    /// The prompt's leader for the keymap `keybindings` holds.
    pub(super) fn new(keybindings: &AppKeybindingsManager) -> Self {
        Self {
            machine: keybindings.leader_machine(Context::Prompt),
            entries: keybindings.leader_entries(),
            label: leader_label(keybindings.leader().id()),
            note: None,
        }
    }

    /// The footer rows at `now`: the pending text, or else a note. The popup
    /// lists the entries once the leader has waited [`WHICH_KEY_DELAY`].
    fn line(&self, now: Instant) -> Option<LeaderLine> {
        if let Some(since) = self.machine.pending_since() {
            let which_key = if now >= since + WHICH_KEY_DELAY {
                self.entries.clone()
            } else {
                Vec::new()
            };
            return Some(LeaderLine {
                text: format!("{} …", self.label),
                which_key,
            });
        }
        let (note, until) = self.note?;
        (now < until).then(|| LeaderLine {
            text: format!("{} … {note}", self.label),
            which_key: Vec::new(),
        })
    }

    /// The times after `now` at which the leader needs a pass: the timeout,
    /// the popup's delay, and the end of a note.
    fn wake_times(&self, now: Instant) -> [Option<Instant>; 3] {
        let popup = self
            .machine
            .pending_since()
            .map(|since| since + WHICH_KEY_DELAY);
        let note = self.note.map(|(_, until)| until);
        [self.machine.deadline(), popup, note].map(|at| at.filter(|at| *at > now))
    }

    fn note(&mut self, text: &'static str, now: Instant) {
        self.note = Some((text, now + NOTE_TIME));
    }

    /// Whether a leader key is waiting for the key after it.
    #[cfg(test)]
    pub(super) fn is_pending(&self) -> bool {
        self.machine.is_pending()
    }
}

impl Mode {
    /// Whether the prompt has the keyboard: no picker, pane or task panel is above it.
    fn prompt_has_focus(&self) -> bool {
        let prompt = self.prompt_component();
        self.tui
            .focused()
            .is_some_and(|focused| Rc::ptr_eq(&focused, &prompt))
    }

    /// Runs one key press through the leader. `true` when the leader took the
    /// key, so it must not reach the screen.
    pub(super) fn leader_takes_key(&mut self, data: &str) -> bool {
        self.leader_takes_key_at(data, Instant::now())
    }

    /// [`Self::leader_takes_key`] for a key that arrived at `now`.
    pub(super) fn leader_takes_key_at(&mut self, data: &str, now: Instant) -> bool {
        // Releases are not presses. The screen drops them too.
        if is_key_release(data) {
            return false;
        }
        if !self.prompt_has_focus() {
            self.leader.machine.reset();
            return false;
        }
        self.leader.note = None;
        match self.leader.machine.handle(now, data) {
            LeaderOutcome::Unhandled => false,
            LeaderOutcome::Pending => true,
            LeaderOutcome::Run(id) => {
                self.run_leader_binding(&id, now);
                true
            }
            LeaderOutcome::Cancelled(CancelReason::Unbound) => {
                self.leader.note("no binding", now);
                true
            }
            LeaderOutcome::Cancelled(_) => true,
        }
    }

    /// Queues the prompt action for a sequence's binding id. An id the prompt
    /// does not answer gets a note instead.
    fn run_leader_binding(&mut self, id: &str, now: Instant) {
        match action_for_binding(id) {
            Some(action) => self.actions.borrow_mut().push(action),
            None => self.leader.note("not here", now),
        }
    }

    /// One pass of the loop on the leader: the timeout, the popup and the
    /// note's end. Repaints when the footer rows change.
    pub(super) fn poll_leader(&mut self) {
        self.poll_leader_at(Instant::now());
    }

    /// [`Self::poll_leader`] at `now`.
    pub(super) fn poll_leader_at(&mut self, now: Instant) {
        // A timeout ends the pending state. The result needs no action.
        let _ = self.leader.machine.poll(now);
        if self.leader.note.is_some_and(|(_, until)| now >= until) {
            self.leader.note = None;
        }
        let line = self.leader.line(now);
        if self.transient.borrow().leader() != line.as_ref() {
            self.transient.borrow_mut().set_leader(line);
            self.dirty.set(true);
        }
    }

    /// The leader's wake times after `now`, for the loop's wait.
    pub(super) fn leader_wake_times(&self, now: Instant) -> [Option<Instant>; 3] {
        self.leader.wake_times(now)
    }
}
