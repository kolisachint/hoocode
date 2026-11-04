//! The quit dialogs (declutter spec, Dialogs): the `Quit?` pane in the
//! prompt's slot. With nothing running it carries the session's totals
//! (`enter` quits, `esc` stays); with background jobs it lists them (`k`
//! stops them and quits, `l` leaves them running, `enter` is stop and quit,
//! `esc` stays).

use hoocode_code_tui_widgets::pane::{hint_line, Pane, PaneStyle};
use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::{Line, Palette, Span};
use hoocode_tui_render::Component;
use hoocode_tui_util::visible_width;

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

    /// The session's totals, dim, after the title.
    fn facts(&self) -> String {
        format!(
            "{} messages · {} tool calls · ↑{} ↓{} · ${:.3}",
            self.debrief.missions,
            self.debrief.steps,
            format_tokens(self.debrief.input_tokens),
            format_tokens(self.debrief.output_tokens),
            self.debrief.cost
        )
    }
}

impl Component for EndShiftCard {
    fn render(&mut self, width: u16) -> Vec<String> {
        let st = PaneStyle::current();
        let mut pane = Pane::new("Quit?");
        pane.facts = Line::from(vec![Span::styled(self.facts(), st.dim_style())]);
        pane.hint = hint_line(&[("enter", "quit"), ("esc", "stay")]);
        pane.style = Some(st);
        pane.render(width as usize)
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

    /// `2 background jobs running`
    fn facts(&self) -> String {
        let n = self.jobs.len();
        let noun = if n == 1 { "job" } else { "jobs" };
        format!("{n} background {noun} running")
    }

    /// The state column: `4s of ~60s`, `4s`, or `done`.
    fn state(job: &ShiftJob) -> String {
        if !job.running {
            return "done".to_string();
        }
        match (job.elapsed_secs, job.total_secs) {
            (Some(elapsed), Some(total)) => format!("{elapsed}s of ~{total}s"),
            (Some(elapsed), None) => format!("{elapsed}s"),
            _ => String::new(),
        }
    }

    /// One row per job, columns aligned: glyph, id (dim), command, state (dim).
    fn rows(&self) -> Vec<Line<'static>> {
        let st = PaneStyle::current();
        let pal = Palette::current();
        let id_w = self
            .jobs
            .iter()
            .map(|j| visible_width(&j.id))
            .max()
            .unwrap_or(0);
        let cmd_w = self
            .jobs
            .iter()
            .map(|j| visible_width(&j.command))
            .max()
            .unwrap_or(0);
        let mut rows: Vec<Line<'static>> = self
            .jobs
            .iter()
            .map(|job| {
                let (mark, mark_style) = if job.running {
                    (status::RUNNING_STR, pal.warn())
                } else {
                    (status::DONE_STR, pal.ok())
                };
                let state = Self::state(job);
                let mut spans = vec![
                    Span::styled(format!("{mark} "), mark_style),
                    Span::styled(pad(&job.id, id_w), st.dim_style()),
                    Span::raw("  "),
                    Span::styled(pad(&job.command, cmd_w), st.text_style()),
                ];
                if !state.is_empty() {
                    spans.push(Span::raw("   "));
                    spans.push(Span::styled(state, st.dim_style()));
                }
                Line::from(spans)
            })
            .collect();
        // One job: where its output stays.
        if let [job] = self.jobs.as_slice() {
            if let Some(path) = job.log_path.as_deref() {
                rows.push(Line::from(vec![Span::styled(
                    format!("  log stays in {path}"),
                    st.dim_style(),
                )]));
            }
        }
        rows
    }
}

impl Component for EndShiftJobsCard {
    fn render(&mut self, width: u16) -> Vec<String> {
        let st = PaneStyle::current();
        let mut pane = Pane::new("Quit?");
        pane.facts = Line::from(vec![Span::styled(self.facts(), st.dim_style())]);
        pane.rows = self.rows();
        pane.hint = hint_line(&[
            ("k", "stop jobs and quit"),
            ("l", "leave them running"),
            ("esc", "stay"),
        ]);
        pane.style = Some(st);
        pane.render(width as usize)
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

/// `text` padded with spaces to `width` cells.
fn pad(text: &str, width: usize) -> String {
    let fill = width.saturating_sub(visible_width(text));
    format!("{text}{}", " ".repeat(fill))
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
