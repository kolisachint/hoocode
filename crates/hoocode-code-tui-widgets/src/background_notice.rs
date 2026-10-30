//! The notice a finished background subagent leaves in the transcript.
//!
//! The text the model receives is unchanged. Only its drawing changes:
//!
//! ```text
//! ◦ explore#4 finished ✓ · 1m27s · 4 still running        radar
//! ◦ explore#4 finished ✓ · 1m27s · 4 still running        peek
//!   │ Mapped the module and its three callers
//!   model haiku-5-5 · effort high
//! ```
//!
//! Text that does not parse is not a notice: `parse_background_notice`
//! returns `None` and the caller keeps the plain rendering.

use hoocode_code_tui_theme::theme;
use hoocode_tui_render::Component;
use hoocode_tui_util::{truncate_to_width, visible_width, wrap_text_with_ansi};
use once_cell::sync::Lazy;
use regex::Regex;

use crate::fold::ELLIPSIS;
use crate::tools::subagent::{elapsed_text, label_color, peek_gutter, strip_markdown};

/// How a background subagent ended, as its notice words it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeVerdict {
    Finished,
    Failed,
    Cancelled,
}

/// A parsed background-subagent notice. Every field but `label` and
/// `verdict` is optional and missing when the text does not carry it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundNotice {
    /// The task's `type#n` label, e.g. `explore#4`.
    pub label: String,
    /// What the run was for, from `explore#4 ("…")`; empty when not given.
    pub description: String,
    /// The Agent call's tool call id, from the header. It finds the run (see
    /// `live::run_for_tool_call`), which a label alone cannot: labels are per
    /// session.
    pub call_id: Option<String>,
    pub verdict: NoticeVerdict,
    /// The task stopped early and can be resumed.
    pub partial: bool,
    /// The one-line summary of the answer; empty when there is none.
    pub summary: String,
    /// Tasks still running when this one settled.
    pub still_running: Option<usize>,
    /// The model's short name, e.g. `haiku-5-5`.
    pub model: Option<String>,
    /// The reasoning effort, e.g. `high`.
    pub effort: Option<String>,
}

/// `Background tool "Agent" (id …) finished:` and the body after it.
static HEADER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)^Background tool "[^"]+" \(id ([^)]*)\) (?:finished|failed):\s*(.*)$"#)
        .expect("background header")
});

/// `explore#4 ("what it was for") finished ✓ (partial …) — summary`, with the
/// description and the summary optional.
static FIRST_LINE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^(\S+#\S+)(?: \("([^"]*)"\))? (finished ✓|failed ✗|cancelled ⊘)( \(partial[^)]*\))?(?: — (.*))?$"#)
        .expect("background first line")
});

/// ` 4 still running.` at the end of the summary.
static STILL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\s(\d+) still running\.$").expect("still running"));

/// `[model: anthropic/claude-haiku-5-5, effort high]`, anywhere in the text.
static MODEL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\[model: ([^,\]]+)(?:, effort ([^\]]+))?\]").expect("model marker"));

/// The model's short name: no provider, no `claude-` prefix.
fn short_model(id: &str) -> String {
    let name = id.trim().rsplit('/').next().unwrap_or(id.trim());
    name.strip_prefix("claude-").unwrap_or(name).to_string()
}

/// Read a background-subagent notice out of a message's text.
///
/// `None` for anything else, including a notice whose first line does not
/// carry a `type#n` label. The "Read the full result" line is dropped here,
/// and so is the model marker, which moves to the peek's own line.
pub fn parse_background_notice(text: &str) -> Option<BackgroundNotice> {
    let header = HEADER_RE.captures(text)?;
    let call_id = header
        .get(1)
        .map(|m| m.as_str().trim().to_string())
        .filter(|id| !id.is_empty());
    let body = header.get(2)?.as_str();

    let (model, effort) = match MODEL_RE.captures(body) {
        Some(caps) => (
            caps.get(1).map(|m| short_model(m.as_str())),
            caps.get(2).map(|m| m.as_str().trim().to_string()),
        ),
        None => (None, None),
    };
    let body = MODEL_RE.replace_all(body, "");

    let first = body.split('\n').next()?.trim();
    let caps = FIRST_LINE_RE.captures(first)?;
    let verdict = match &caps[3] {
        "finished ✓" => NoticeVerdict::Finished,
        "failed ✗" => NoticeVerdict::Failed,
        _ => NoticeVerdict::Cancelled,
    };
    let rest = caps.get(5).map_or("", |m| m.as_str()).trim();
    let (rest, still_running) = match STILL_RE.captures(rest) {
        Some(caps) => (
            STILL_RE.replace(rest, "").into_owned(),
            caps[1].parse::<usize>().ok(),
        ),
        None => (rest.to_string(), None),
    };
    let summary = rest.trim();
    let summary = summary.strip_suffix('.').unwrap_or(summary).trim();

    Some(BackgroundNotice {
        label: caps[1].to_string(),
        description: caps.get(2).map_or("", |m| m.as_str()).to_string(),
        call_id,
        verdict,
        partial: caps.get(4).is_some(),
        summary: summary.to_string(),
        still_running,
        model,
        effort,
    })
}

/// The notice as the transcript draws it: one line at radar, plus the
/// summary and the model at peek.
pub struct BackgroundNoticeComponent {
    notice: BackgroundNotice,
    elapsed: Option<String>,
    expanded: bool,
    full: bool,
}

impl BackgroundNoticeComponent {
    /// The elapsed time comes from the subagent inbox, when it still knows
    /// the task.
    pub fn new(notice: BackgroundNotice) -> Self {
        let elapsed = notice
            .call_id
            .as_deref()
            .and_then(|call_id| elapsed_text(call_id, &notice.label));
        Self::with_elapsed(notice, elapsed)
    }

    pub fn with_elapsed(notice: BackgroundNotice, elapsed: Option<String>) -> Self {
        Self {
            notice,
            elapsed,
            expanded: true,
            full: false,
        }
    }

    /// `true` at peek (the summary and model show), `false` at radar.
    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    /// `true` at the full stop: the summary wraps whole instead of being cut
    /// to one row.
    pub fn set_full(&mut self, full: bool) {
        self.full = full;
    }

    fn head_line(&self) -> String {
        let t = theme();
        let notice = &self.notice;
        let (word, tone) = match notice.verdict {
            NoticeVerdict::Finished => ("finished ✓", "success"),
            NoticeVerdict::Failed => ("✗ failed", "error"),
            NoticeVerdict::Cancelled => ("⊘ cancelled", "dim"),
        };
        let mut line = format!(
            "{} {} {}",
            t.fg(tone, "◦"),
            t.fg(label_color(&notice.label, "accent"), &notice.label),
            t.fg(tone, word),
        );
        if notice.partial {
            line.push_str(&t.fg("warning", " (partial)"));
        }
        let mut parts: Vec<String> = Vec::new();
        if let Some(elapsed) = &self.elapsed {
            parts.push(elapsed.clone());
        }
        if let Some(n) = notice.still_running.filter(|n| *n > 0) {
            parts.push(format!("{n} still running"));
        }
        for part in parts {
            line.push_str(&t.fg("dim", " · "));
            line.push_str(&t.fg("muted", &part));
        }
        line
    }
}

impl Component for BackgroundNoticeComponent {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        let width = width as usize;
        let t = theme();
        let mut lines = vec![self.head_line()];
        if self.expanded {
            let summary = strip_markdown(&self.notice.summary);
            if !summary.is_empty() {
                let gutter = peek_gutter();
                let inner = width.saturating_sub(visible_width(&gutter)).max(1);
                if self.full {
                    for row in wrap_text_with_ansi(&summary, inner) {
                        lines.push(format!("{gutter}{}", t.fg("toolOutput", &row)));
                    }
                } else {
                    let shown = truncate_to_width(&summary, inner, ELLIPSIS, false);
                    lines.push(format!("{gutter}{}", t.fg("toolOutput", &shown)));
                }
            }
            if let Some(model) = &self.notice.model {
                let mut line = format!("model {model}");
                if let Some(effort) = &self.notice.effort {
                    line.push_str(&format!(" · effort {effort}"));
                }
                lines.push(format!("  {}", t.fg("dim", &line)));
            }
        }
        if self.full {
            return lines
                .into_iter()
                .flat_map(|line| wrap_text_with_ansi(&line, width.max(1)))
                .collect();
        }
        lines
            .into_iter()
            .map(|line| truncate_to_width(&line, width, ELLIPSIS, false))
            .collect()
    }
}
