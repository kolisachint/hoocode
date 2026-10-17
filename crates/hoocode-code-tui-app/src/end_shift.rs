//! The Mission Control end-of-shift cards (M6, spec §4 c8 and the "Jettison,
//! end of shift, hull paint" card): the amber cards that replace the comms
//! frame when a quit is asked. With nothing running, the card carries the
//! shift's debrief (`enter` ends the shift, `esc` stays); with satellites up,
//! it lists them (`k` brings them down and ends, `l` leaves them in orbit,
//! `enter` is the default: bring them down).

use hoocode_code_tui_keybindings::raw_key_hint;
use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::card_frame::render_card;
use hoocode_tui_render::Component;

/// The plain end-of-shift card's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndShiftChoice {
    End,
    Stay,
}

/// What the end-of-shift card answers with.
pub type EndShiftAnswer = Box<dyn FnOnce(EndShiftChoice)>;

/// The shift's debrief, from the session's real totals.
#[derive(Debug, Clone, PartialEq)]
pub struct ShiftDebrief {
    /// User messages (missions flown).
    pub missions: usize,
    /// Tool calls (steps taken).
    pub steps: usize,
    /// Cumulative input/output tokens.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Money spent.
    pub cost: f64,
}

/// The end-of-shift card: the debrief, then `enter` ends, `esc` stays.
pub struct EndShiftCard {
    debrief: ShiftDebrief,
    on_done: Option<EndShiftAnswer>,
}

impl EndShiftCard {
    /// The card for `debrief`; `on_done` answers exactly once.
    pub fn new(debrief: ShiftDebrief, on_done: EndShiftAnswer) -> Self {
        Self {
            debrief,
            on_done: Some(on_done),
        }
    }

    fn done(&mut self, choice: EndShiftChoice) {
        if let Some(answer) = self.on_done.take() {
            answer(choice);
        }
    }

    fn body(&self) -> Vec<String> {
        let t = theme();
        let sep = t.fg("muted", " · ");
        let debrief = [
            t.fg("text", &format!("{} missions", self.debrief.missions)),
            t.fg("text", &format!("{} steps", self.debrief.steps)),
            t.fg(
                "dim",
                &format!(
                    "↑{} ↓{}",
                    format_tokens(self.debrief.input_tokens),
                    format_tokens(self.debrief.output_tokens)
                ),
            ),
            t.fg("dim", &format!("CR ${:.3}", self.debrief.cost)),
        ]
        .join(&sep);
        vec![
            debrief,
            String::new(),
            [
                raw_key_hint("enter", "end the shift"),
                raw_key_hint("esc", "stay"),
            ]
            .join(&sep),
        ]
    }
}

impl Component for EndShiftCard {
    fn render(&mut self, width: u16) -> Vec<String> {
        render_card(
            "◉ END OF SHIFT",
            "esc stay",
            &self.body(),
            "",
            width as usize,
        )
    }

    fn handle_input(&mut self, data: &str) {
        match data {
            "\r" | "\n" => self.done(EndShiftChoice::End),
            "\x1b" => self.done(EndShiftChoice::Stay),
            _ => {}
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }
}

/// One satellite on the jobs card: its id, command and state word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftJob {
    pub id: String,
    pub command: String,
    pub running: bool,
}

/// The jobs card's answer. It keeps the old prompt's outcomes: the default
/// brings every job down, `l` leaves them in orbit, esc stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndShiftJobsChoice {
    KillAll,
    LeaveRunning,
    Stay,
}

/// What the jobs card answers with.
pub type EndShiftJobsAnswer = Box<dyn FnOnce(EndShiftJobsChoice)>;

/// The end-of-shift-with-satellites card (spec §4 c8).
pub struct EndShiftJobsCard {
    jobs: Vec<ShiftJob>,
    on_done: Option<EndShiftJobsAnswer>,
}

impl EndShiftJobsCard {
    /// The card for `jobs`; `on_done` answers exactly once.
    pub fn new(jobs: Vec<ShiftJob>, on_done: EndShiftJobsAnswer) -> Self {
        Self {
            jobs,
            on_done: Some(on_done),
        }
    }

    fn done(&mut self, choice: EndShiftJobsChoice) {
        if let Some(answer) = self.on_done.take() {
            answer(choice);
        }
    }

    /// `◉ END SHIFT WITH 1 SATELLITE UP?`
    fn title(&self) -> String {
        let n = self.jobs.len();
        let noun = if n == 1 { "SATELLITE" } else { "SATELLITES" };
        format!("◉ END SHIFT WITH {n} {noun} UP?")
    }

    fn body(&self) -> Vec<String> {
        let t = theme();
        let mut rows = Vec::new();
        for job in &self.jobs {
            let mark = if job.running {
                t.fg("accent", "◐")
            } else {
                t.fg("dim", "●")
            };
            let state = if job.running {
                t.fg("dim", "in orbit")
            } else {
                t.fg("dim", "done")
            };
            rows.push(format!(
                "  {mark} {}  {}   {state}",
                t.fg("text", &job.id),
                t.fg("text", &job.command),
            ));
        }
        rows.push(String::new());
        let sep = t.fg("muted", " · ");
        rows.push(
            [
                raw_key_hint("k", "bring it down, then end shift"),
                raw_key_hint("l", "leave it in orbit"),
            ]
            .join(&sep),
        );
        rows
    }
}

impl Component for EndShiftJobsCard {
    fn render(&mut self, width: u16) -> Vec<String> {
        render_card(&self.title(), "esc stay", &self.body(), "", width as usize)
    }

    fn handle_input(&mut self, data: &str) {
        match data {
            // The old default stays: enter brings every job down.
            "\r" | "\n" | "k" => self.done(EndShiftJobsChoice::KillAll),
            "l" => self.done(EndShiftJobsChoice::LeaveRunning),
            "\x1b" => self.done(EndShiftJobsChoice::Stay),
            _ => {}
        }
    }

    fn is_focusable(&self) -> bool {
        true
    }
}

/// Compact token counts (`48.2k`), like the comms foot's totals.
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}
