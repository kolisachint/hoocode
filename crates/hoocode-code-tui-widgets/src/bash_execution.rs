//! `components/bash-execution.ts`: a user `!` command with streaming output,
//! a spinner while it runs, and a status line once it ends.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use hoocode_code_tool_api::{
    truncate_tail, TruncationOptions, TruncationResult, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES,
};
use hoocode_code_tool_bash::shell::strip_ansi;
use hoocode_code_tui_theme::theme;
use hoocode_tui_components::Loader;
use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{bold, Palette};
use hoocode_tui_render::Component;
use hoocode_tui_util::wrap_text_with_ansi;

use crate::fold::more_lines_hint;
use crate::trajectory::{rail_line, wrapped_band_rows};

/// Preview line limit for a `!` command at peek; the full stop shows every line.
const PREVIEW_LINES: usize = 20;

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
    /// The transcript's radar flag, shared so a view flip reaches every row.
    radar: Option<Rc<Cell<bool>>>,
    /// The transcript's full flag: at the full stop the output shows whole
    /// and the command is never cut.
    full: Option<Rc<Cell<bool>>>,
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
            radar: None,
            full: None,
        }
    }

    /// Radar (M5): while `flag` is set the row shows its header only, the
    /// output stays in peek. The flag is the transcript's radar flag.
    pub fn set_radar_flag(&mut self, flag: Rc<Cell<bool>>) {
        self.radar = Some(flag);
    }

    fn radar_hidden(&self) -> bool {
        self.radar.as_ref().is_some_and(|flag| flag.get())
    }

    /// The transcript's full flag, shared so a view flip reaches every row.
    pub fn set_full_flag(&mut self, flag: Rc<Cell<bool>>) {
        self.full = Some(flag);
    }

    /// Whether the output shows whole: the full stop, or `set_expanded`.
    fn is_expanded(&self) -> bool {
        self.expanded || self.full.as_ref().is_some_and(|flag| flag.get())
    }

    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    /// Advance the spinner; true when it changed.
    pub fn tick(&mut self) -> bool {
        self.status == Status::Running && self.loader.borrow_mut().tick()
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

    /// The `▲ $` band header (Mission Control spec a4): the mark in the
    /// `you` token, the command in bold ink, and — once settled — the exit
    /// on the right with `· no step, it's yours`. Your own `!` commands
    /// are yours, not the agent's steps, so they get no step number.
    ///
    /// The command is never cut: a long one wraps across continuation rows.
    fn header_rows(&self, width: usize, palette: &Palette) -> Vec<String> {
        let prefix = vec![
            Span::styled("▲ $".to_string(), bold(palette.you())),
            Span::styled("  ".to_string(), Style::default()),
        ];
        wrapped_band_rows(
            prefix,
            &self.command,
            bold(palette.ink()),
            self.header_stats(palette),
            width,
            palette.bg_color("userMessageBg"),
        )
    }

    /// The header's right side: the exit (ok/err, warn when cancelled)
    /// plus `· no step, it's yours`. Empty while the command runs.
    fn header_stats(&self, palette: &Palette) -> Vec<Span<'static>> {
        let (exit, style) = match self.status {
            Status::Running => return Vec::new(),
            Status::Complete => (
                format!("exit {}", self.exit_code.unwrap_or(0)),
                palette.ok(),
            ),
            Status::Error => (
                format!(
                    "exit {}",
                    self.exit_code
                        .map_or("undefined".to_string(), |c| c.to_string())
                ),
                palette.err(),
            ),
            Status::Cancelled => ("cancelled".to_string(), palette.warn()),
        };
        vec![
            Span::styled(exit, style),
            Span::styled(" · ".to_string(), palette.dim()),
            Span::styled("no step, it's yours".to_string(), palette.dim()),
        ]
    }

    /// The output and footer rows: dim output on the `┊` rail, then the
    /// spinner while running, else the fold/cancelled/truncation notices.
    fn body_rows(&mut self, width: usize, palette: &Palette) -> Vec<String> {
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
            // The rail takes six cells; the output wraps in what is left.
            let content_width = width.saturating_sub(6).max(1);
            let mut visual = Vec::new();
            for physical in context.content.split('\n') {
                visual.extend(wrap_text_with_ansi(&t.fg("dim", physical), content_width));
            }
            let shown = if self.is_expanded() {
                visual.as_slice()
            } else {
                let skip = visual.len().saturating_sub(PREVIEW_LINES);
                hidden = skip;
                &visual[skip..]
            };
            for line in shown {
                lines.push(rail_line(line, width, false, palette));
            }
        }
        if self.status == Status::Running {
            lines.extend(self.loader.borrow_mut().render(width as u16));
            return lines;
        }
        let mut footer: Vec<String> = Vec::new();
        if hidden > 0 {
            footer.push(rail_line(&more_lines_hint(hidden), width, false, palette));
        }
        if self.status == Status::Cancelled {
            footer.push(rail_line(
                &t.fg("warning", "(cancelled)"),
                width,
                false,
                palette,
            ));
        }
        let truncated =
            self.truncation.as_ref().is_some_and(|tr| tr.truncated) || context.truncated;
        if let (true, Some(path)) = (truncated, &self.full_output_path) {
            footer.push(rail_line(
                &t.fg("warning", &format!("Output truncated. Full output: {path}")),
                width,
                false,
                palette,
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
        let mut lines = self.header_rows(width, &palette);
        if !self.radar_hidden() {
            lines.extend(self.body_rows(width, &palette));
        }
        lines
    }

    fn invalidate(&mut self) {
        self.loader.borrow_mut().invalidate();
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
