//! The transient rows above the prompt: the runtime warning (memory shedding,
//! UI stall), the auto-retry notice, extension statuses and startup progress.
//! Each is one row cut to the width, dim or yellow, no fill. The two-row
//! footer that used to carry them is gone (declutter: the status line under
//! the prompt holds the session facts).

use hoocode_tui_ratatui::{paint, Palette};
use hoocode_tui_render::Component;
use hoocode_tui_util::truncate_to_width;

use crate::footer_data::FooterDataProvider;
use crate::progress_bar::{render_download_progress, render_progress_bar};
use crate::startup_progress::{self, StartupProgress};

/// Newlines, tabs and carriage returns become spaces; runs of spaces collapse.
pub(crate) fn sanitize_status_text(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| {
            if matches!(c, '\r' | '\n' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut out = String::new();
    for c in replaced.chars() {
        if c == ' ' && out.ends_with(' ') {
            continue;
        }
        out.push(c);
    }
    out.trim().to_string()
}

/// One row for a startup progress entry.
fn render_startup_line(entry: &StartupProgress) -> String {
    let pal = Palette::current();
    match entry {
        StartupProgress::Error { label, message, .. } => {
            paint(pal.dim(), &format!("{label}: {message}"))
        }
        StartupProgress::Download {
            label,
            received_bytes,
            total_bytes,
            ..
        } => format!(
            "{} {}",
            paint(pal.dim(), label),
            render_download_progress(*received_bytes, *total_bytes, None)
        ),
        StartupProgress::Work {
            label,
            done,
            total,
            unit,
            ..
        } => {
            let ratio = if *total > 0 {
                *done as f64 / *total as f64
            } else {
                0.0
            };
            format!(
                "{} {}",
                paint(pal.dim(), label),
                render_progress_bar(ratio, &format!("{done}/{total} {unit}"), None)
            )
        }
    }
}

/// The transient rows: the warning, then the retry notice, then extension
/// statuses, then startup progress. Indent 1, at most `width` cells each.
pub struct TransientLines {
    data: FooterDataProvider,
    /// A warning (memory shedding, UI stall).
    notice: Option<String>,
    /// The auto-retry notice while the session retries a transient failure
    /// (`retrying · attempt 2 of 6 · …`).
    retry_notice: Option<String>,
}

impl TransientLines {
    pub fn new(data: FooterDataProvider) -> Self {
        Self {
            data,
            notice: None,
            retry_notice: None,
        }
    }

    /// Sets or clears the warning. `None` removes it.
    pub fn set_notice(&mut self, notice: Option<String>) {
        self.notice = notice;
    }

    /// Sets or clears the auto-retry notice. `None` removes it.
    pub fn set_retry_notice(&mut self, notice: Option<String>) {
        self.retry_notice = notice;
    }

    /// The rows at `width`.
    pub fn lines(&self, width: usize) -> Vec<String> {
        let pal = Palette::current();
        let room = width.saturating_sub(1).max(1);
        let cut = |text: &str, ellipsis: &str| {
            format!(
                " {}",
                truncate_to_width(text, room, &paint(pal.dim(), ellipsis), false)
            )
        };
        let mut lines = Vec::new();
        if let Some(notice) = &self.notice {
            lines.push(cut(&paint(pal.warn(), notice), "…"));
        }
        if let Some(notice) = &self.retry_notice {
            lines.push(cut(&paint(pal.warn(), &sanitize_status_text(notice)), "…"));
        }
        let statuses = self.data.get_extension_statuses();
        if !statuses.is_empty() {
            let line = statuses
                .values()
                .map(|text| sanitize_status_text(text))
                .collect::<Vec<_>>()
                .join(" ");
            lines.push(cut(&paint(pal.dim(), &line), "…"));
        }
        for entry in startup_progress::list() {
            lines.push(cut(&render_startup_line(&entry), "…"));
        }
        lines
    }
}

impl Component for TransientLines {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.lines(width as usize)
    }
}
