//! `components/bash-execution.ts`: a user `!` command with streaming output,
//! a spinner while it runs, and a status line once it ends.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use hoocode_code_tool_api::{
    truncate_tail, TruncationOptions, TruncationResult, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES,
};
use hoocode_code_tool_bash::shell::strip_ansi;
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::Loader;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{ansi_spans, row, Palette};
use hoocode_tui_render::{Component, Fold, FoldHost};
use hoocode_tui_util::{truncate_to_width, wrap_text_with_ansi};

use crate::fold::{decorate_header, earlier_lines_text, is_fold_line, FoldState, INDICATOR_CELLS};
use crate::tool_row::{dim_fact, fold_row, is_blank_row, tool_rows, Mark, BODY_INDENT};

/// Output lines a passed `!` command shows at peek (its last), a failed one,
/// and the length up to which output shows whole. The full stop shows all.
const PASSED_LINES: usize = 2;
const FAILED_LINES: usize = 4;
const WHOLE_LINES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Running,
    Complete,
    Cancelled,
    Error,
}

/// Lines the tail keeps: one more than the context keeps, so a dropped line
/// still reads as truncated.
const KEPT_LINES: usize = DEFAULT_MAX_LINES + 1;
/// Bytes the tail keeps: twice the context's byte limit, for the same reason.
const KEPT_BYTES: usize = 2 * DEFAULT_MAX_BYTES;

/// The tail of a streamed command output. Only the last lines reach the
/// context, so the rest is dropped as it arrives. Each update then joins a
/// bounded buffer, not the whole output.
#[derive(Debug, Default)]
struct OutputTail {
    lines: VecDeque<String>,
    /// Whether a line or byte before the kept tail was dropped.
    dropped: bool,
}

impl OutputTail {
    fn append(&mut self, chunk: &str) {
        let clean = strip_ansi(chunk).replace("\r\n", "\n").replace('\r', "\n");
        let mut new_lines = clean.split('\n').map(String::from);
        if let Some(last) = self.lines.back_mut() {
            if let Some(first) = new_lines.next() {
                last.push_str(&first);
            }
        }
        self.lines.extend(new_lines);
        self.trim();
    }

    /// Drop what the context can never show. The kept tail still truncates
    /// to the same text: the context walks back at most `DEFAULT_MAX_LINES`
    /// lines and `DEFAULT_MAX_BYTES` bytes, and both fit in what is kept.
    fn trim(&mut self) {
        while self.lines.len() > KEPT_LINES {
            self.lines.pop_front();
            self.dropped = true;
        }
        // A single line longer than the byte bound keeps its end.
        if let Some(last) = self.lines.back_mut() {
            if last.len() > KEPT_BYTES {
                let mut start = last.len() - KEPT_BYTES;
                while !last.is_char_boundary(start) {
                    start += 1;
                }
                last.drain(..start);
                self.dropped = true;
            }
        }
        let mut bytes: usize = self.lines.iter().map(|l| l.len() + 1).sum();
        while bytes > KEPT_BYTES && self.lines.len() > 1 {
            if let Some(line) = self.lines.pop_front() {
                bytes -= line.len() + 1;
            }
            self.dropped = true;
        }
    }

    fn text(&self) -> String {
        self.lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub struct BashExecutionComponent {
    command: String,
    status: Status,
    exit_code: Option<i32>,
    loader: Rc<RefCell<Loader>>,
    truncation: Option<TruncationResult>,
    full_output_path: Option<String>,
    expanded: bool,
    tail: OutputTail,
    fold: FoldState,
    /// The rows of the last render that the block's header and fold line
    /// took, for the fold host.
    last_header: Vec<usize>,
    last_fold_line: Option<usize>,
}

impl BashExecutionComponent {
    /// `excludeFromContext` (`!!`) draws the spinner dim instead of in the
    /// bash-mode colour.
    pub fn new(command: &str, exclude_from_context: bool) -> Self {
        let color_key: &'static str = if exclude_from_context {
            "dim"
        } else {
            "bashMode"
        };
        let mut loader = Loader::new(
            Box::new(move |s: &str| theme().fg(color_key, s)),
            Box::new(|s: &str| theme().fg("muted", s)),
            "",
            None,
        );
        loader.start();
        Self {
            command: command.to_string(),
            tail: OutputTail::default(),
            status: Status::Running,
            exit_code: None,
            loader: Rc::new(RefCell::new(loader)),
            truncation: None,
            full_output_path: None,
            expanded: false,
            fold: FoldState::new("shell"),
            last_header: Vec::new(),
            last_fold_line: None,
        }
    }

    /// Whether opening shows anything: the command has printed output.
    fn can_open(&self) -> bool {
        !self.tail.text().trim().is_empty()
    }

    /// Whether the output shows whole: the block's own toggle. Collapsed,
    /// a running command shows one dim tail line.
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    /// Advance the spinner; true when it changed.
    pub fn tick(&mut self) -> bool {
        self.status == Status::Running && self.loader.borrow_mut().tick()
    }

    /// When the spinner's next frame falls due, or `None` when it is not running.
    /// The UI loop waits no longer than this, so the frame is not late.
    pub fn next_deadline(&self) -> Option<std::time::Instant> {
        if self.status != Status::Running {
            return None;
        }
        self.loader.borrow().next_deadline()
    }

    pub fn append_output(&mut self, chunk: &str) {
        self.tail.append(chunk);
    }

    pub fn set_complete(
        &mut self,
        exit_code: Option<i32>,
        cancelled: bool,
        truncation: Option<TruncationResult>,
        full_output_path: Option<String>,
    ) {
        self.exit_code = exit_code;
        self.status = if cancelled {
            Status::Cancelled
        } else if exit_code.is_some_and(|c| c != 0) {
            Status::Error
        } else {
            Status::Complete
        };
        self.truncation = truncation;
        self.full_output_path = full_output_path;
        self.loader.borrow_mut().stop();
    }

    /// The command's tool row: ` ● ! command  exit 0`. A failed run reads
    /// `⊗` with `exit N` in red, a cancelled one a dim `⊗ cancelled`, a running
    /// one a dim `●`. The command is never cut: a long one wraps under itself.
    fn header_rows(&self, width: usize, palette: &Palette) -> Vec<String> {
        let (mark, facts) = match self.status {
            Status::Running => (Mark::Running, Vec::new()),
            Status::Complete => (Mark::Done, vec![dim_fact("exit 0", palette)]),
            Status::Error => (
                Mark::Failed,
                vec![Span::styled(
                    format!(
                        "exit {}",
                        self.exit_code
                            .map_or("undefined".to_string(), |c| c.to_string())
                    ),
                    palette.err(),
                )],
            ),
            Status::Cancelled => (Mark::Stopped, vec![dim_fact("cancelled", palette)]),
        };
        let t = theme();
        let label = format!(
            "{} {}",
            t.fg("toolTitle", &t.bold("!")),
            t.fg("text", &self.command)
        );
        tool_rows(0, mark, &label, facts, width, palette)
    }

    /// A running command's single dim tail line while collapsed (UI-21 P4,
    /// decision 4): its last output line, dim at column 4. Gone once the
    /// command settles. With no output yet, the spinner stands in.
    fn running_tail_line(&mut self, width: usize) -> Vec<String> {
        let t = theme();
        let content_width = width.saturating_sub(BODY_INDENT).max(1);
        let last = self
            .tail
            .text()
            .lines()
            .rfind(|l| !l.trim().is_empty())
            .map(str::to_string);
        match last {
            Some(line) => {
                let cut = truncate_to_width(&line, content_width, "…", false);
                let text = format!("{}{}", " ".repeat(BODY_INDENT), t.fg("dim", &cut));
                vec![row(ansi_spans(&text), width)]
            }
            None => self.spinner_rows(width),
        }
    }

    /// The spinner rows at column 4, without blank rows.
    fn spinner_rows(&mut self, width: usize) -> Vec<String> {
        self.loader
            .borrow_mut()
            .render(width.saturating_sub(BODY_INDENT).max(1) as u16)
            .into_iter()
            .filter(|l| !is_blank_row(l))
            .map(|l| format!("{}{l}", " ".repeat(BODY_INDENT)))
            .collect()
    }
    /// The output rows, dim at column 4: the last two lines of a run that
    /// passed, the last four of one that failed, whole when it is four lines
    /// or fewer and at the full stop. The fold hint sits above the lines;
    /// the cancelled and truncation notices close the body. A running
    /// command, collapsed, shows one dim tail line instead (see
    /// [`Self::running_tail_line`]); expanded, its tail and spinner.
    fn body_rows(&mut self, width: usize, palette: &Palette) -> Vec<String> {
        if self.status == Status::Running && !self.is_expanded() {
            return self.running_tail_line(width);
        }
        let t = theme();
        let mut lines = Vec::new();
        let mut context = truncate_tail(
            &self.tail.text(),
            TruncationOptions {
                max_lines: Some(DEFAULT_MAX_LINES),
                max_bytes: Some(DEFAULT_MAX_BYTES),
            },
        );
        context.truncated |= self.tail.dropped;
        let mut hidden = 0;
        if !context.content.is_empty() {
            // The body column takes four cells; the output wraps in what is left.
            let content_width = width.saturating_sub(BODY_INDENT).max(1);
            let mut visual = Vec::new();
            for physical in context.content.trim_end().split('\n') {
                visual.extend(wrap_text_with_ansi(&t.fg("dim", physical), content_width));
            }
            let budget = if self.status == Status::Error {
                FAILED_LINES
            } else {
                PASSED_LINES
            };
            let shown = if self.is_expanded() || visual.len() <= WHOLE_LINES {
                visual.as_slice()
            } else {
                let skip = visual.len().saturating_sub(budget);
                hidden = skip;
                &visual[skip..]
            };
            if hidden > 0 {
                lines.push(fold_row(&earlier_lines_text(hidden), width, palette));
            }
            for line in shown {
                lines.push(row(
                    vec![Span::raw(" ".repeat(BODY_INDENT))]
                        .into_iter()
                        .chain(ansi_spans(line))
                        .collect(),
                    width,
                ));
            }
        }
        if self.status == Status::Running {
            lines.extend(self.spinner_rows(width));
            return lines;
        }
        let mut footer: Vec<String> = Vec::new();
        if self.status == Status::Cancelled {
            footer.push(row(
                vec![
                    Span::raw(" ".repeat(BODY_INDENT)),
                    Span::styled("(cancelled)".to_string(), palette.warn()),
                ],
                width,
            ));
        }
        let truncated =
            self.truncation.as_ref().is_some_and(|tr| tr.truncated) || context.truncated;
        if let (true, Some(path)) = (truncated, &self.full_output_path) {
            footer.push(row(
                vec![
                    Span::raw(" ".repeat(BODY_INDENT)),
                    Span::styled(
                        format!("Output truncated. Full output: {path}"),
                        palette.warn(),
                    ),
                ],
                width,
            ));
        }
        lines.extend(footer);
        lines
    }

    /// The output kept for display: the last lines of it. The earlier ones
    /// are dropped as they stream in.
    pub fn output(&self) -> String {
        self.tail.text()
    }

    pub fn command(&self) -> &str {
        &self.command
    }
}

impl Component for BashExecutionComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        let palette = Palette::current();
        let width = (width as usize).max(1);
        // One blank row opens the run, as it does a chain of tool rows.
        let mut lines = vec![String::new()];
        // A command that can open ends its header in the indicator, whose
        // two cells come off the header's room first.
        let can_open = self.can_open();
        let reserve = if can_open { INDICATOR_CELLS } else { 0 };
        let header = self.header_rows(width.saturating_sub(reserve).max(1), &palette);
        let header_at: Vec<usize> = (1..=header.len()).collect();
        lines.extend(header);
        let body = self.body_rows(width, &palette);
        self.last_fold_line = (!self.expanded)
            .then(|| body.iter().position(|row| is_fold_line(row)))
            .flatten()
            .map(|at| lines.len() + at);
        if !body.is_empty() {
            lines.extend(body);
        }
        decorate_header(
            &mut lines,
            &header_at,
            can_open,
            self.expanded,
            self.fold.focused(),
            width,
            &palette,
        );
        self.last_header = header_at;
        lines
    }

    fn fold_host(&self) -> Option<&dyn FoldHost> {
        Some(self)
    }

    fn fold_host_mut(&mut self) -> Option<&mut dyn FoldHost> {
        Some(self)
    }

    fn invalidate(&mut self) {
        self.loader.borrow_mut().invalidate();
    }
}

impl FoldHost for BashExecutionComponent {
    fn folds(&self) -> Vec<Fold> {
        vec![Fold {
            id: self.fold.id().to_string(),
            label: "Shell".to_string(),
            header: self.last_header.clone(),
            fold_line: self.last_fold_line,
            expanded: self.expanded,
            can_open: self.can_open(),
        }]
    }

    fn set_fold_expanded(&mut self, id: &str, expanded: bool) -> bool {
        if !self.fold.is(id) {
            return false;
        }
        self.set_expanded(expanded);
        true
    }

    fn set_fold_focus(&mut self, id: Option<&str>) {
        self.fold.set_focus(id);
    }

    fn fold_text(&self, id: &str) -> Option<String> {
        self.fold.is(id).then(|| self.output())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> TruncationOptions {
        TruncationOptions {
            max_lines: Some(DEFAULT_MAX_LINES),
            max_bytes: Some(DEFAULT_MAX_BYTES),
        }
    }

    /// The context from the whole output, as the component showed it before
    /// the tail: every chunk cleaned, then truncated.
    fn full_context(chunks: &[String]) -> TruncationResult {
        let full: String = chunks
            .iter()
            .map(|c| strip_ansi(c).replace("\r\n", "\n").replace('\r', "\n"))
            .collect();
        truncate_tail(&full, options())
    }

    /// The context from the tail, with the same flag the component reads.
    fn tail_context(tail: &OutputTail) -> TruncationResult {
        let mut context = truncate_tail(&tail.text(), options());
        context.truncated |= tail.dropped;
        context
    }

    /// Feed each chunk and compare with the whole output after every step.
    fn assert_tail_matches_full(chunks: &[String]) {
        let mut tail = OutputTail::default();
        for (i, chunk) in chunks.iter().enumerate() {
            tail.append(chunk);
            let expected = full_context(&chunks[..=i]);
            let actual = tail_context(&tail);
            assert_eq!(actual.content, expected.content, "content after chunk {i}");
            assert_eq!(actual.truncated, expected.truncated, "flag after chunk {i}");
        }
    }

    #[test]
    fn tail_matches_full_output_for_many_short_lines() {
        let chunks: Vec<String> = (0..2600).map(|i| format!("line {i}\n")).collect();
        assert_tail_matches_full(&chunks);
    }

    #[test]
    fn tail_matches_full_output_at_the_line_limit() {
        for lines in [
            DEFAULT_MAX_LINES - 1,
            DEFAULT_MAX_LINES,
            DEFAULT_MAX_LINES + 1,
            DEFAULT_MAX_LINES + 2,
        ] {
            let chunks: Vec<String> = (0..lines).map(|i| format!("{i}\n")).collect();
            assert_tail_matches_full(&chunks);
        }
    }

    #[test]
    fn tail_matches_full_output_when_bytes_are_the_limit() {
        // 900 lines of 100 bytes: under the line limit, over the byte limit.
        let chunks: Vec<String> = (0..900).map(|i| format!("{i:0>99}\n")).collect();
        assert_tail_matches_full(&chunks);
    }

    #[test]
    fn tail_matches_full_output_for_one_huge_line_in_pieces() {
        let chunks: Vec<String> = (0..100).map(|_| "0123456789abcdef".repeat(64)).collect();
        assert_tail_matches_full(&chunks);
        let mut chunks: Vec<String> = (0..60).map(|_| "z".repeat(3000)).collect();
        chunks.push("\n".into());
        chunks.push("tail\n".into());
        assert_tail_matches_full(&chunks);
    }

    #[test]
    fn tail_keeps_a_character_boundary_when_it_cuts_a_long_line() {
        // Two-byte characters: the byte cut must land between them.
        let chunks: Vec<String> = (0..50).map(|_| "é".repeat(1500)).collect();
        assert_tail_matches_full(&chunks);
    }

    #[test]
    fn tail_matches_full_output_with_ansi_and_carriage_returns() {
        let chunks: Vec<String> = (0..900)
            .map(|i| format!("\u{1b}[31mred {i}\u{1b}[0m\r\n{i}\r"))
            .collect();
        assert_tail_matches_full(&chunks);
    }

    #[test]
    fn tail_memory_stays_bounded_over_a_long_stream() {
        let mut tail = OutputTail::default();
        for i in 0..10_000 {
            tail.append(&format!("{i:08} {}\n", "x".repeat(1000)));
        }
        assert!(tail.lines.len() <= KEPT_LINES);
        assert!(tail.text().len() <= KEPT_BYTES + KEPT_LINES);
        assert!(tail.dropped, "a long stream drops its head");
    }

    #[test]
    fn short_output_is_kept_whole() {
        let mut tail = OutputTail::default();
        tail.append("a\nb");
        tail.append("c\n");
        assert_eq!(tail.text(), "a\nbc\n");
        assert!(!tail.dropped);
    }
}
