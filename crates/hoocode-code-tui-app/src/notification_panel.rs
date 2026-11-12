//! The transient band above the prompt (`components/notification-panel.ts`).
//!
//! A glimpse (`Info`) reports state, so only the newest is true and it
//! replaces the queued one; a warning reports an event, so warnings queue.
//! Two glimpses with the same `topic` (a dial stepped twice) replace each
//! other even on screen, and restart the clock.
//!
//! hoocode runs one timer; here the owner polls: [`NotificationPanel::poll`]
//! expires what is due and [`NotificationPanel::deadline`] says when to poll
//! next. The clock is injectable for tests.
//!
//! A notice with no body rows is one row: it shows in the status line's left
//! slot ([`NotificationPanel::status_notice`]) and the band stays empty. A
//! longer one is drawn here, above the prompt, as plain rows: one blank row,
//! then rows indented by one column with no fill (a warning's first row
//! ink bold; the rest dim).
//!
//! A notice that needs an action ([`NotificationPanel::notify_action`])
//! never times out; [`NotificationPanel::dismiss`] removes it once the action
//! is done.

use std::rc::Rc;
use std::time::{Duration, Instant};

use hoocode_code_tui_theme::theme;
use hoocode_code_tui_widgets::pane::{Pane, PaneStyle};
use hoocode_tui_ratatui::{Line, Span};
use hoocode_tui_render::Component;

use crate::status_line::{KeyHint, NoticeLevel, StatusNotice};
use hoocode_tui_util::{truncate_to_width, visible_width, wrap_text_with_ansi};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    Info,
    Warning,
}

impl NotificationKind {
    /// `NOTIFICATION_TTL_MS`.
    pub fn ttl(self) -> Duration {
        match self {
            NotificationKind::Info => Duration::from_millis(3000),
            NotificationKind::Warning => Duration::from_millis(8000),
        }
    }
}

/// Reading time per body row.
const BODY_ROW: Duration = Duration::from_millis(1200);
const MAX_TTL: Duration = Duration::from_millis(30_000);
const MAX_BODY_ROWS: usize = 3;
const MAX_QUEUE: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    pub kind: NotificationKind,
    pub title: String,
    pub body: Vec<String>,
    pub note: Option<String>,
    pub ttl: Option<Duration>,
    pub topic: Option<String>,
    /// Needs the user to act: never times out.
    pub action: bool,
}

/// A string that already carries escapes keeps its own colour.
fn ink(color: &str, text: &str) -> String {
    if text.contains("\x1b[") {
        text.to_string()
    } else {
        theme().fg(color, text)
    }
}

/// A title as one row: styling dropped, line breaks as spaces.
fn one_line(text: &str) -> String {
    hoocode_tui_util::strip_vt_control_characters(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

type Clock = Rc<dyn Fn() -> Instant>;

pub struct NotificationPanel {
    queue: Vec<Notification>,
    deadline: Option<Instant>,
    request_render: Box<dyn Fn()>,
    max_body_rows: Box<dyn Fn() -> usize>,
    clock: Clock,
    cache: Option<(usize, usize, Vec<String>)>,
    /// The status line's left slot is free for a one-row notice (no turn
    /// is running): such a notice shows there and the band stays empty.
    status_slot_free: bool,
    /// The task panel is drawn right above the band: the band then joins
    /// its pane (see [`Self::set_joined_to_pane`]).
    joined_to_pane: bool,
}

impl NotificationPanel {
    pub fn new(
        request_render: impl Fn() + 'static,
        max_body_rows: Option<Box<dyn Fn() -> usize>>,
    ) -> Self {
        Self::with_clock(request_render, max_body_rows, Rc::new(Instant::now))
    }

    pub fn with_clock(
        request_render: impl Fn() + 'static,
        max_body_rows: Option<Box<dyn Fn() -> usize>>,
        clock: Clock,
    ) -> Self {
        Self {
            queue: Vec::new(),
            deadline: None,
            request_render: Box::new(request_render),
            max_body_rows: max_body_rows.unwrap_or_else(|| Box::new(|| MAX_BODY_ROWS)),
            clock,
            cache: None,
            status_slot_free: true,
            joined_to_pane: false,
        }
    }

    /// The notification on screen.
    pub fn showing(&self) -> Option<&Notification> {
        self.queue.first()
    }

    /// Those waiting behind it.
    pub fn pending(&self) -> &[Notification] {
        self.queue.get(1..).unwrap_or(&[])
    }

    /// When the head expires (poll then).
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    pub fn notify(
        &mut self,
        kind: NotificationKind,
        title: &str,
        body: &[&str],
        note: Option<&str>,
        ttl: Option<Duration>,
        topic: Option<&str>,
    ) {
        self.push(kind, title, body, note, ttl, topic, false);
    }

    /// A notice that needs an action: it stays until [`Self::dismiss`] (or a
    /// notice with the same `topic` replaces it).
    pub fn notify_action(
        &mut self,
        kind: NotificationKind,
        title: &str,
        body: &[&str],
        note: Option<&str>,
        topic: &str,
    ) {
        self.push(kind, title, body, note, None, Some(topic), true);
    }

    /// Remove the notice with `topic`, shown or queued.
    pub fn dismiss(&mut self, topic: &str) {
        let Some(i) = self
            .queue
            .iter()
            .position(|n| n.topic.as_deref() == Some(topic))
        else {
            return;
        };
        self.queue.remove(i);
        if i == 0 {
            self.deadline = None;
            let now = (self.clock)();
            self.arm(now);
        }
        self.cache = None;
        (self.request_render)();
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: NotificationKind,
        title: &str,
        body: &[&str],
        note: Option<&str>,
        ttl: Option<Duration>,
        topic: Option<&str>,
        action: bool,
    ) {
        let next = Notification {
            kind,
            title: title.to_string(),
            body: body
                .iter()
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect(),
            note: note.map(str::to_string),
            ttl,
            topic: topic.map(str::to_string),
            action,
        };
        let same_topic = next
            .topic
            .as_ref()
            .and_then(|t| self.queue.iter().position(|n| n.topic.as_ref() == Some(t)));
        let tail = self.queue.len().checked_sub(1);
        if let Some(i) = same_topic {
            self.queue[i] = next;
            if i == 0 {
                let now = (self.clock)();
                self.arm(now);
            }
        } else if kind == NotificationKind::Info
            && tail.is_some_and(|t| t > 0 && self.queue[t].kind == NotificationKind::Info)
        {
            let t = tail.unwrap_or(0);
            self.queue[t] = next;
        } else {
            if self.queue.len() >= MAX_QUEUE {
                self.queue.remove(1);
            }
            self.queue.push(next);
            if self.queue.len() == 1 {
                let now = (self.clock)();
                self.arm(now);
            }
        }
        self.cache = None;
        (self.request_render)();
    }

    /// Stop the clock (shutdown).
    pub fn stop(&mut self) {
        self.deadline = None;
    }

    /// Expire every notification whose time is up; true when one went.
    pub fn poll(&mut self) -> bool {
        let now = (self.clock)();
        let mut expired = false;
        while let Some(deadline) = self.deadline {
            if now < deadline {
                break;
            }
            self.deadline = None;
            if !self.queue.is_empty() {
                self.queue.remove(0);
            }
            self.cache = None;
            if !self.queue.is_empty() {
                // The next one's clock starts when the previous one expired.
                self.arm(deadline);
            }
            (self.request_render)();
            expired = true;
        }
        expired
    }

    fn budget(&self) -> usize {
        (self.max_body_rows)().max(1)
    }

    fn body_rows<'a>(&self, current: &'a Notification) -> &'a [String] {
        &current.body[..current.body.len().min(self.budget())]
    }

    fn ttl(&self, current: &Notification) -> Duration {
        if let Some(ttl) = current.ttl {
            return ttl;
        }
        (current.kind.ttl() + BODY_ROW * self.body_rows(current).len() as u32).min(MAX_TTL)
    }

    /// Whether the status line's left slot is free for a one-row notice:
    /// false while a turn runs (`◉ working` keeps the slot), and the notice
    /// is drawn in the band instead.
    pub fn set_status_slot_free(&mut self, free: bool) {
        if self.status_slot_free != free {
            self.status_slot_free = free;
            self.cache = None;
            (self.request_render)();
        }
    }

    /// Whether the task panel is drawn right above the band. Then the band's
    /// rows go on the pane colour, full width, at the pane's indent 3, with no
    /// blank row above, so a tip reads as the last rows of the task pane. In
    /// monochrome there is no fill and the band renders as before.
    pub fn set_joined_to_pane(&mut self, joined: bool) {
        if self.joined_to_pane != joined {
            self.joined_to_pane = joined;
            self.cache = None;
            (self.request_render)();
        }
    }

    /// The notice up now as a status-line notice, when it is one row (no
    /// body). Its `note` (`shift+alt+m back`) becomes the key hint: the first
    /// word the key, the rest what it does. The panel's own clock expires it.
    pub fn status_notice(&self) -> Option<StatusNotice> {
        let current = self
            .queue
            .first()
            .filter(|n| n.body.is_empty() && self.status_slot_free)?;
        let level = match current.kind {
            NotificationKind::Info => NoticeLevel::Info,
            NotificationKind::Warning => NoticeLevel::Warning,
        };
        let mut notice = StatusNotice::new(level, one_line(&current.title));
        if let Some(note) = current
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            let (key, action) = split_note(note);
            notice.keys.push(KeyHint::new(key, action));
        }
        Some(notice)
    }

    fn arm(&mut self, from: Instant) {
        self.deadline = self
            .queue
            .first()
            .filter(|current| !current.action)
            .map(|current| from + self.ttl(current));
    }
}

/// Splits a note into its key and its action. The key is the first word,
/// or the chord `ctrl+x n` (a modifier word and the single key after it),
/// so the whole key reads as ink. The rest is the action.
fn split_note(note: &str) -> (String, String) {
    let words: Vec<&str> = note.splitn(3, ' ').collect();
    let chord = words.len() == 3 && words[0].contains("ctrl+") && words[1].len() == 1;
    let take = if chord { 2 } else { 1 };
    let mut parts = note.splitn(take + 1, ' ');
    let key: Vec<&str> = (0..take).filter_map(|_| parts.next()).collect();
    let action = parts.next().unwrap_or("").trim();
    (key.join(" "), action.to_string())
}

impl Component for NotificationPanel {
    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let Some(current) = self.queue.first() else {
            return Vec::new();
        };
        if width < 4 {
            return Vec::new();
        }
        // One row: the status line shows it while its slot is free.
        if current.body.is_empty() && self.status_slot_free {
            return Vec::new();
        }
        if self.joined_to_pane {
            // Monochrome keeps the same rows: only the fill is dropped.
            return self.render_joined(current, width, PaneStyle::current());
        }
        let rows = self.budget();
        if let Some((w, r, lines)) = &self.cache {
            if *w == width && *r == rows {
                return lines.clone();
            }
        }
        let inner = (width - 1).max(1);
        let warning = current.kind == NotificationKind::Warning;
        let styled_title = |row: &str| {
            if warning {
                let text = ink("ink", row);
                if !row.contains("\x1b[") {
                    theme().bold(&text)
                } else {
                    text
                }
            } else {
                ink("dim", row)
            }
        };
        // The title wraps at spaces (at most 3 rows, the last ends in `…`);
        // the key hint joins the last row when it fits, else takes its own.
        let mut title_rows = wrap_text_with_ansi(&one_line(&current.title), inner);
        if title_rows.len() > 3 {
            title_rows.truncate(3);
            let last = title_rows.pop().unwrap_or_default();
            title_rows.push(truncate_to_width(&format!("{last}…"), inner, "…", false));
        }
        let mut lines = vec![String::new()];
        let hint = current
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(split_note);
        let hint_text = |(key, action): &(String, String)| {
            let mut out = theme().fg("text", key);
            if !action.is_empty() {
                out.push(' ');
                out.push_str(&theme().fg("dim", action));
            }
            out
        };
        let last = title_rows.len().saturating_sub(1);
        for (i, row) in title_rows.iter().enumerate() {
            let mut line = format!(" {}", styled_title(row));
            if i == last {
                if let Some(hint) = &hint {
                    let plain = format!("{}  {} {}", row, hint.0, hint.1);
                    if visible_width(plain.trim_end()) <= inner {
                        line.push_str("  ");
                        line.push_str(&hint_text(hint));
                    } else if lines.len() < 4 {
                        lines.push(line);
                        line = format!(" {}", hint_text(hint));
                    }
                }
            }
            lines.push(line);
        }
        for line in self.body_rows(current) {
            lines.push(format!(
                " {}",
                ink("dim", &truncate_to_width(line, inner, "…", false))
            ));
        }
        self.cache = Some((width, rows, lines.clone()));
        lines
    }

    fn invalidate(&mut self) {
        self.cache = None;
    }
}

impl NotificationPanel {
    /// The band as the task pane's last rows: the pane's background, full
    /// width, rows at indent 3, no blank row above. Drawn through [`Pane`], so
    /// the fill is the pane's own. Not cached: the pane style is read per draw.
    fn render_joined(&self, current: &Notification, width: usize, st: PaneStyle) -> Vec<String> {
        let inner = width.saturating_sub(4).max(1);
        let warning = current.kind == NotificationKind::Warning;
        let title_style = if warning {
            st.text_style()
                .add_modifier(hoocode_tui_ratatui::Modifier::BOLD)
        } else {
            st.dim_style()
        };
        let mut title_rows = wrap_text_with_ansi(&one_line(&current.title), inner);
        if title_rows.len() > 3 {
            title_rows.truncate(3);
            let last = title_rows.pop().unwrap_or_default();
            title_rows.push(truncate_to_width(&format!("{last}…"), inner, "…", false));
        }
        let hint = current
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(split_note);
        let hint_spans = |(key, action): &(String, String)| {
            let mut spans = vec![Span::styled(key.clone(), st.text_style())];
            if !action.is_empty() {
                spans.push(Span::styled(format!(" {action}"), st.dim_style()));
            }
            spans
        };
        // Each row is one line of spans; the hint joins the last title row
        // when it fits, else takes a row of its own.
        let mut rows: Vec<Vec<Span<'static>>> = Vec::new();
        let last = title_rows.len().saturating_sub(1);
        for (i, row) in title_rows.iter().enumerate() {
            let mut spans = vec![Span::styled(row.clone(), title_style)];
            if let (true, Some(hint)) = (i == last, &hint) {
                let plain = format!("{row}  {} {}", hint.0, hint.1);
                if visible_width(plain.trim_end()) <= inner {
                    spans.push(Span::raw("  "));
                    spans.extend(hint_spans(hint));
                } else {
                    rows.push(spans);
                    spans = hint_spans(hint);
                }
            }
            rows.push(spans);
        }
        for line in self.body_rows(current) {
            let text = truncate_to_width(line, inner, "…", false);
            rows.push(vec![Span::styled(text, st.dim_style())]);
        }
        let lines: Vec<Line<'static>> = rows
            .into_iter()
            .map(|spans| {
                let mut line = vec![Span::raw("  ")];
                line.extend(spans);
                Line::from(line)
            })
            .collect();
        if st.mono {
            // A monochrome pane would draw a border; the band keeps its
            // rows and drops only the fill, so it stays borderless.
            return lines
                .iter()
                .map(|line| format!(" {}", hoocode_tui_ratatui::flatten(line)))
                .collect();
        }
        let mut pane = Pane::new("");
        pane.top = lines;
        pane.style = Some(st);
        pane.render(width)
    }
}
