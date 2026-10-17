//! The Mission Control end-of-shift cards (M6, spec §4 c8 and the "Jettison,
//! end of shift, hull paint" card): the amber cards that replace the comms
//! frame when a quit is asked. With nothing running, the card carries the
//! shift's debrief (`enter` ends the shift, `esc` stays); with satellites up,
//! the `◎ END SHIFT WITH 1 SATELLITE UP?` card lists them with their orbit
//! clocks (`k` brings them down and ends, `l` leaves them in orbit, `enter`
//! is the default: bring them down).

use hoocode_code_tui_keybindings::format_key_text;
use hoocode_code_tui_widgets::card_frame::{card_foot, render_card};
use hoocode_tui_ratatui::{Line, Palette, Span};
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

    fn body(&self) -> Vec<Line<'static>> {
        let pal = Palette::current();
        let sep = Span::styled(" · ", pal.faint());
        let debrief = [
            Span::styled(
                format!("{} missions", self.debrief.missions),
                pal.legacy("text"),
            ),
            Span::styled(format!("{} steps", self.debrief.steps), pal.legacy("text")),
            Span::styled(
                format!(
                    "↑{} ↓{}",
                    format_tokens(self.debrief.input_tokens),
                    format_tokens(self.debrief.output_tokens)
                ),
                pal.dim(),
            ),
            Span::styled(format!("CR ${:.3}", self.debrief.cost), pal.dim()),
        ];
        let mut first = Line::from(vec![debrief[0].clone()]);
        for span in &debrief[1..] {
            let mut spans = first.spans.clone();
            spans.push(sep.clone());
            spans.push(span.clone());
            first = Line::from(spans);
        }
        vec![
            first,
            Line::raw(""),
            hints_line(&[("enter", "end the shift"), ("esc", "stay")]),
        ]
    }
}

impl Component for EndShiftCard {
    fn render(&mut self, width: u16) -> Vec<String> {
        render_card(
            "◉ END OF SHIFT",
            "esc stay",
            &self.body(),
            &card_foot(""),
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

/// One satellite on the jobs card: its id, command and state word. The
/// orbit clock (`elapsed_secs` of `~total_secs`) and the log path render
/// when the host knows them (spec §4 c8: `in orbit 4s of ~60s`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShiftJob {
    pub id: String,
    pub command: String,
    pub running: bool,
    /// Seconds in orbit, when known.
    pub elapsed_secs: Option<u64>,
    /// Rough total (a `sleep N` command's `N`), when known.
    pub total_secs: Option<u64>,
    /// Where its output stays, when known (`jobs/job-1.log`).
    pub log_path: Option<String>,
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

    /// `◎ END SHIFT WITH 1 SATELLITE UP?` The ring (`◎`) is the card's own
    /// glyph, shared with the clearance card.
    fn title(&self) -> String {
        let n = self.jobs.len();
        let noun = if n == 1 { "SATELLITE" } else { "SATELLITES" };
        format!("◎ END SHIFT WITH {n} {noun} UP?")
    }

    fn body(&self) -> Vec<Line<'static>> {
        let pal = Palette::current();
        let mut rows = Vec::new();
        for job in &self.jobs {
            let (mark, mark_style) = if job.running {
                ("◐", pal.acc())
            } else {
                ("●", pal.dim())
            };
            let mut state = if job.running {
                "in orbit".to_string()
            } else {
                "done".to_string()
            };
            // The orbit clock, when the host knows it: `in orbit 4s of ~60s`.
            if job.running {
                match (job.elapsed_secs, job.total_secs) {
                    (Some(elapsed), Some(total)) => {
                        state = format!("in orbit {elapsed}s of ~{total}s");
                    }
                    (Some(elapsed), None) => {
                        state = format!("in orbit {elapsed}s");
                    }
                    _ => {}
                }
            }
            rows.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(mark, mark_style),
                Span::raw(" "),
                Span::styled(job.id.clone(), pal.legacy("text")),
                Span::raw("  "),
                Span::styled(job.command.clone(), pal.legacy("text")),
                Span::raw("   "),
                Span::styled(state, pal.dim()),
            ]));
        }
        rows.push(Line::raw(""));
        let mut hints: Vec<(&str, &str)> = vec![
            ("k", "bring it down, then end shift"),
            ("l", "leave it in orbit"),
        ];
        // One satellite: the foot names where its log stays (spec §4 c8).
        let log_note;
        if self.jobs.len() == 1 {
            if let Some(path) = self.jobs[0].log_path.as_deref() {
                log_note = format!("log stays in {path}");
                hints.push(("", log_note.as_str()));
            }
        }
        rows.push(hints_line(&hints));
        rows
    }
}

impl Component for EndShiftJobsCard {
    fn render(&mut self, width: u16) -> Vec<String> {
        render_card(
            &self.title(),
            "esc stay",
            &self.body(),
            &card_foot(""),
            width as usize,
        )
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

/// One hints row: dim keys plus muted descriptions, joined by `·`.
fn hints_line(hints: &[(&str, &str)]) -> Line<'static> {
    let pal = Palette::current();
    let mut spans = Vec::new();
    for (i, (key, description)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", pal.faint()));
        }
        spans.push(Span::styled(format_key_text(key, false), pal.dim()));
        spans.push(Span::styled(format!(" {description}"), pal.faint()));
    }
    Line::from(spans)
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
