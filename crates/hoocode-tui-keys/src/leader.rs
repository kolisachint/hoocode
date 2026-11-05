//! The leader key: a pending state for sequences that start with one key
//! (Ctrl+X by default). Pure logic, no UI. Time is passed in as an
//! `Instant`, so tests do not sleep.
//!
//! Flow: the leader enters pending. The next key either finishes a bound
//! sequence (`Run`), extends it (`Pending`), or cancels it (`Cancelled`).
//! While pending, every key is swallowed, so none reaches the editor. A key
//! that arrives after the timeout is not swallowed: the pending state has
//! ended, and the key is handled as an ordinary key.

use std::time::{Duration, Instant};

use crate::matching::KeySpec;
use crate::sequence::{KeySequence, KeyStep, SequenceError};

/// How long a pending leader waits for the next key, unless configured.
pub const DEFAULT_LEADER_TIMEOUT: Duration = Duration::from_millis(1500);

/// Why a leader key is not usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaderError {
    /// The text is not a key identifier.
    NotAKey(String),
    /// The key types a character (`m`, `/`, `shift+a`, `space`). Typing
    /// would be swallowed, so it cannot be the leader.
    Printable(String),
}

impl std::fmt::Display for LeaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAKey(key) => write!(f, "not a key: {key:?}"),
            Self::Printable(key) => write!(
                f,
                "{key:?} types a character; the leader needs a key with ctrl, alt or super, or a named key"
            ),
        }
    }
}

impl std::error::Error for LeaderError {}

/// A validated leader key. It cannot be a printable key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderKey {
    step: KeyStep,
}

impl LeaderKey {
    /// Validates a key id such as `"ctrl+x"`.
    pub fn new(text: &str) -> Result<Self, LeaderError> {
        let step = KeyStep::parse(text).map_err(|_| LeaderError::NotAKey(text.to_string()))?;
        if step.is_printable() {
            return Err(LeaderError::Printable(text.to_string()));
        }
        Ok(Self { step })
    }

    /// The key id as written.
    pub fn id(&self) -> &str {
        self.step.id()
    }
}

/// What the machine did with one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaderOutcome {
    /// Not the leader, and no sequence is pending. The caller handles the key.
    Unhandled,
    /// Swallowed. The leader is pending, or the sequence needs more keys.
    Pending,
    /// A bound sequence finished. Run the action with this id.
    Run(String),
    /// The pending sequence ended without a binding. The key is swallowed.
    Cancelled(CancelReason),
}

/// Why a pending leader ended without running anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// Esc was pressed.
    Escape,
    /// The leader was pressed again.
    LeaderAgain,
    /// The next key has no binding. The UI reports "no binding".
    Unbound,
    /// The timeout passed before the next key (reported by [`LeaderMachine::poll`]).
    Timeout,
}

struct Binding {
    steps: Vec<KeyStep>,
    action: String,
}

struct Pending {
    /// Steps consumed so far, the leader included.
    depth: usize,
    /// Indices of the bindings that still match every step so far.
    candidates: Vec<usize>,
    deadline: Instant,
}

/// Leader state and the sequence table it reads.
pub struct LeaderMachine {
    leader: LeaderKey,
    escape: KeySpec,
    timeout: Duration,
    bindings: Vec<Binding>,
    pending: Option<Pending>,
}

impl LeaderMachine {
    /// A machine with no bindings. `timeout` is how long a pending leader waits.
    pub fn new(leader: LeaderKey, timeout: Duration) -> Self {
        let escape = KeySpec::parse("escape").unwrap_or_else(|| unreachable!("escape parses"));
        Self {
            leader,
            escape,
            timeout,
            bindings: Vec::new(),
            pending: None,
        }
    }

    /// The leader key.
    pub fn leader(&self) -> &LeaderKey {
        &self.leader
    }

    /// The wait for the next key.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Binds `sequence` (for example `"ctrl+x m"`) to `action`. A sequence
    /// that is already bound is replaced. The sequence must start with the
    /// leader, have at least one step after it, and must not use the leader
    /// or Esc after the first step.
    pub fn bind(&mut self, sequence: &str, action: &str) -> Result<(), SequenceError> {
        let parsed = KeySequence::parse(sequence)?;
        let steps = parsed.steps();
        if steps[0].spec() != self.leader.step.spec() {
            return Err(SequenceError::NoLeader(sequence.to_string()));
        }
        if steps.len() < 2 {
            return Err(SequenceError::LeaderOnly(sequence.to_string()));
        }
        let reserved = steps[1..]
            .iter()
            .any(|step| step.spec() == self.leader.step.spec() || step.spec() == &self.escape);
        if reserved {
            return Err(SequenceError::ReservedStep(sequence.to_string()));
        }
        let binding = Binding {
            steps: steps.to_vec(),
            action: action.to_string(),
        };
        match self.bindings.iter_mut().find(|b| b.steps == binding.steps) {
            Some(existing) => existing.action = binding.action,
            None => self.bindings.push(binding),
        }
        Ok(())
    }

    /// Whether a leader is waiting for its next key.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Steps consumed by the pending sequence, the leader included. Zero when idle.
    pub fn pending_depth(&self) -> usize {
        self.pending.as_ref().map_or(0, |p| p.depth)
    }

    /// When the pending sequence times out, if one is pending.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|p| p.deadline)
    }

    /// Feeds one key press. `now` is the time it arrived.
    pub fn handle(&mut self, now: Instant, data: &str) -> LeaderOutcome {
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            // Timed out. The key is handled as a fresh key. `poll` reports it.
            self.pending = None;
        }
        match self.pending.take() {
            None => {
                if self.leader.step.matches(data) {
                    self.pending = Some(Pending {
                        depth: 1,
                        candidates: (0..self.bindings.len()).collect(),
                        deadline: now + self.timeout,
                    });
                    LeaderOutcome::Pending
                } else {
                    LeaderOutcome::Unhandled
                }
            }
            Some(pending) => self.advance(now, pending, data),
        }
    }

    /// Checks the timeout without a key. Returns `Some(Timeout)` once when
    /// the pending sequence has expired, and ends it.
    pub fn poll(&mut self, now: Instant) -> Option<CancelReason> {
        if self.pending.as_ref().is_some_and(|p| now >= p.deadline) {
            self.pending = None;
            return Some(CancelReason::Timeout);
        }
        None
    }

    /// Ends any pending sequence without a report (for example, when focus
    /// leaves the prompt).
    pub fn reset(&mut self) {
        self.pending = None;
    }

    fn advance(&mut self, now: Instant, pending: Pending, data: &str) -> LeaderOutcome {
        if self.escape.matches(data) {
            return LeaderOutcome::Cancelled(CancelReason::Escape);
        }
        if self.leader.step.matches(data) {
            return LeaderOutcome::Cancelled(CancelReason::LeaderAgain);
        }
        let depth = pending.depth;
        let candidates: Vec<usize> = pending
            .candidates
            .into_iter()
            .filter(|&i| {
                self.bindings[i]
                    .steps
                    .get(depth)
                    .is_some_and(|step| step.matches(data))
            })
            .collect();
        if candidates.is_empty() {
            return LeaderOutcome::Cancelled(CancelReason::Unbound);
        }
        let depth = depth + 1;
        // An exact match runs at once, even if a longer sequence shares its prefix.
        if let Some(&done) = candidates
            .iter()
            .find(|&&i| self.bindings[i].steps.len() == depth)
        {
            return LeaderOutcome::Run(self.bindings[done].action.clone());
        }
        self.pending = Some(Pending {
            depth,
            candidates,
            deadline: now + self.timeout,
        });
        LeaderOutcome::Pending
    }
}
