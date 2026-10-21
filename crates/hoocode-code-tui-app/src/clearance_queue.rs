//! Which clearance card is up, and who waits behind it.
//!
//! One card fits the editor's slot. The main agent and any number of
//! subagents (relayed from their children) may ask at once, so a request that
//! arrives while a card is up waits its turn instead of denying the card that
//! holds the slot. Requests are shown in the order they arrived; a request
//! withdrawn while it waits (its subagent ended) is dropped.

use std::collections::VecDeque;
use std::sync::mpsc;

use hoocode_code_permissions::{ClearanceChoice, ClearanceInfo};

/// The id of a request that is never withdrawn: the main agent's own calls.
pub const OWN_CALL: u64 = 0;

/// One request for a card: what to show, where the answer goes, and the id the
/// asker can withdraw it by.
pub struct PendingClearance {
    pub id: u64,
    pub info: ClearanceInfo,
    pub reply: mpsc::Sender<ClearanceChoice>,
}

/// Where a withdrawn request was.
pub enum Withdrawn {
    /// Its card is up now: the caller closes it (which shows the next).
    Open,
    /// It was still waiting and is gone from the queue.
    Waiting(Box<PendingClearance>),
    /// Not known (already answered).
    Unknown,
}

/// The card in the slot (by id) and the requests behind it.
#[derive(Default)]
pub struct ClearanceQueue {
    open: Option<u64>,
    waiting: VecDeque<PendingClearance>,
}

impl ClearanceQueue {
    /// A request arrives. `Some` means its card is up now and the caller shows
    /// it; `None` means another card holds the slot and the request waits.
    pub fn arrive(&mut self, request: PendingClearance) -> Option<PendingClearance> {
        if self.open.is_some() {
            self.waiting.push_back(request);
            return None;
        }
        self.open = Some(request.id);
        Some(request)
    }

    /// The open card was answered or closed. Returns the next request to show.
    pub fn closed(&mut self) -> Option<PendingClearance> {
        self.open = None;
        let next = self.waiting.pop_front()?;
        self.open = Some(next.id);
        Some(next)
    }

    /// The asker no longer wants an answer.
    pub fn withdraw(&mut self, id: u64) -> Withdrawn {
        if id == OWN_CALL {
            return Withdrawn::Unknown;
        }
        if self.open == Some(id) {
            return Withdrawn::Open;
        }
        let at = self.waiting.iter().position(|request| request.id == id);
        match at.and_then(|at| self.waiting.remove(at)) {
            Some(request) => Withdrawn::Waiting(Box::new(request)),
            None => Withdrawn::Unknown,
        }
    }

    /// Everything waiting, in order, with the queue left empty. The open card
    /// (if any) is the caller's to answer.
    pub fn drain(&mut self) -> Vec<PendingClearance> {
        self.open = None;
        self.waiting.drain(..).collect()
    }

    /// How many requests wait behind the open card.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }
}
