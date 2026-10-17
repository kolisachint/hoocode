//! Transmission toasts (M8 e4): notices as round toast overlays.
//!
//! A notice is a transmission: a round frame (`╭ ╮ ╰ ╯`) floating top right
//! under the HUD, over the transcript, 46 cells wide, titled `●
//! TRANSMISSION`. A `▔` drain row shrinks as the toast times out. The newest
//! toast shows; the queue behind it waits its turn.
//!
//! The rows are drawn with ratatui ([`transmission_component`], pure runs
//! below it, tested here).
//! [`NotificationPanel`](crate::notification_panel::NotificationPanel) feeds
//! the newest notification in and the fullscreen frame floats the lines; the
//! panel's own band goes quiet while the overlay is up.

use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::{Line, Span};
use hoocode_tui_ratatui::{bold, Palette, RatatuiComponent};
use hoocode_tui_util::{strip_vt_control_characters, truncate_to_width, visible_width};

/// The toast width in cells (spec §5.3).
pub const TOAST_WIDTH: usize = 46;

/// At most this many body rows under the title; the band budgets the same.
pub const TOAST_BODY_ROWS: usize = 3;

/// What one toast shows: the notice title first, then its body rows. `note`
/// rides the title row (`shift+alt+z undo`). `remaining` is the share of its
/// time left (0 to 1) and drives the `▔` drain row.
#[derive(Debug, Clone, PartialEq)]
pub struct TransmissionData {
    pub title: String,
    pub body: Vec<String>,
    pub note: Option<String>,
    pub remaining: f32,
}

/// One styled run of a toast row: plain text plus the palette token styling
/// it. `""` is the default style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransmissionRun {
    pub text: String,
    pub token: &'static str,
    pub bold: bool,
}

fn run(text: String, token: &'static str) -> TransmissionRun {
    TransmissionRun {
        text,
        token,
        bold: false,
    }
}

fn style_run(run: &TransmissionRun, pal: &Palette) -> Style {
    let style = pal.style(run.token);
    if run.bold {
        bold(style)
    } else {
        style
    }
}

/// The content between the frame edges, padded with blanks to `inner` cells.
fn padded(mut runs: Vec<TransmissionRun>, inner: usize) -> Vec<TransmissionRun> {
    let used: usize = runs.iter().map(|r| visible_width(&r.text)).sum();
    if used < inner {
        runs.push(run(" ".repeat(inner - used), ""));
    }
    runs
}

/// The title row: `╭ ● TRANSMISSION  note ╮`, the note dropped first when
/// the row runs short. Exactly `width` cells.
fn title_row(note: Option<&str>, width: usize) -> Vec<TransmissionRun> {
    // The edges take three cells (`╭ ` and `╮`); the note gets what fits.
    let inner = width.saturating_sub(3);
    let mut content = vec![
        run("●".to_string(), "acc"),
        TransmissionRun {
            text: " TRANSMISSION".to_string(),
            token: "ink",
            bold: true,
        },
    ];
    if let Some(note) = note.filter(|n| !n.is_empty()) {
        let used: usize = content.iter().map(|r| visible_width(&r.text)).sum();
        if used + 2 + visible_width(note) <= inner {
            content.push(run("  ".to_string(), ""));
            content.push(run(note.to_string(), "dim"));
        }
    }
    let mut row = vec![run("╭ ".to_string(), "faint")];
    row.extend(padded(content, inner));
    row.push(run("╮".to_string(), "faint"));
    row
}

/// One body row: `│ text │`, cut with `…` when long. Escape codes the notice
/// painted itself do not survive the float: the toast is one shape.
fn body_row(body: &str, width: usize) -> Vec<TransmissionRun> {
    let inner = width.saturating_sub(4);
    let plain = strip_vt_control_characters(body);
    let text = truncate_to_width(plain.trim_end(), inner, "…", false);
    let mut row = vec![run("│ ".to_string(), "faint")];
    row.extend(padded(vec![run(text, "ink")], inner));
    row.push(run(" │".to_string(), "faint"));
    row
}

/// The drain row: `│ ▔▔… │`, the bar shrinking with `remaining`.
fn drain_row(remaining: f32, width: usize) -> Vec<TransmissionRun> {
    let inner = width.saturating_sub(4);
    let cells = (remaining.clamp(0.0, 1.0) * inner as f32).round() as usize;
    let mut row = vec![run("│ ".to_string(), "faint")];
    row.extend(padded(
        vec![run("▔".repeat(cells.min(inner)), "acc")],
        inner,
    ));
    row.push(run(" │".to_string(), "faint"));
    row
}

/// The bottom edge: `╰ ─…─ ╯`. Exactly `width` cells.
fn bottom_row(width: usize) -> Vec<TransmissionRun> {
    vec![run(
        format!("╰{}╯", "─".repeat(width.saturating_sub(2))),
        "faint",
    )]
}

/// The toast as run rows, each exactly `width` cells: the title, the notice
/// title and up to two more body rows, the drain row, and the bottom edge.
/// `width` under a frameable minimum gives no rows.
pub fn transmission_runs(data: &TransmissionData, width: usize) -> Vec<Vec<TransmissionRun>> {
    if width < 12 {
        return Vec::new();
    }
    let mut rows = vec![title_row(data.note.as_deref(), width)];
    let mut body = vec![data.title.clone()];
    body.extend(data.body.iter().cloned());
    for line in body.iter().take(TOAST_BODY_ROWS) {
        rows.push(body_row(line, width));
    }
    rows.push(drain_row(data.remaining, width));
    rows.push(bottom_row(width));
    rows
}

/// The toast as a component `width` cells wide.
pub fn transmission_component(
    data: TransmissionData,
    width: usize,
) -> RatatuiComponent<impl FnMut(Rect, &mut Buffer)> {
    let rows = transmission_runs(&data, width);
    let height = rows.len() as u16;
    RatatuiComponent::new(height, move |area: Rect, buf: &mut Buffer| {
        let pal = Palette::current();
        for (y, row) in rows.iter().enumerate() {
            let spans: Vec<Span<'static>> = row
                .iter()
                .map(|r| Span::styled(r.text.clone(), style_run(r, &pal)))
                .collect();
            buf.set_line(area.x, area.y + y as u16, &Line::from(spans), area.width);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> TransmissionData {
        TransmissionData {
            title: "chrome is now compact".to_string(),
            body: vec![],
            note: Some("shift+alt+z undo".to_string()),
            remaining: 1.0,
        }
    }

    fn plain(rows: &[Vec<TransmissionRun>]) -> Vec<String> {
        rows.iter()
            .map(|row| row.iter().map(|r| r.text.as_str()).collect())
            .collect()
    }

    #[test]
    fn toast_is_46_wide_with_a_round_frame_and_drain() {
        let rows = plain(&transmission_runs(&data(), TOAST_WIDTH));
        assert_eq!(rows.len(), 4, "{rows:?}");
        for row in &rows {
            assert_eq!(visible_width(row), TOAST_WIDTH, "{row:?}");
        }
        assert!(rows[0].starts_with("╭ ● TRANSMISSION"), "{:?}", rows[0]);
        assert!(rows[0].ends_with('╮'), "{:?}", rows[0]);
        assert!(rows[0].contains("shift+alt+z undo"), "{:?}", rows[0]);
        assert!(rows[1].contains("chrome is now compact"), "{:?}", rows[1]);
        assert_eq!(
            rows[2].matches('▔').count(),
            TOAST_WIDTH - 4,
            "{:?}",
            rows[2]
        );
        assert!(
            rows[3].starts_with('╰') && rows[3].ends_with('╯'),
            "{:?}",
            rows[3]
        );
    }

    #[test]
    fn drain_shrinks_with_the_time_left() {
        let mut half = data();
        half.remaining = 0.5;
        let rows = plain(&transmission_runs(&half, TOAST_WIDTH));
        assert_eq!(
            rows[2].matches('▔').count(),
            (TOAST_WIDTH - 4) / 2,
            "{:?}",
            rows[2]
        );
        let mut spent = data();
        spent.remaining = 0.0;
        let rows = plain(&transmission_runs(&spent, TOAST_WIDTH));
        assert!(!rows[2].contains('▔'), "{:?}", rows[2]);
    }

    #[test]
    fn long_note_is_dropped_before_the_title() {
        let mut data = data();
        data.note = Some("a note far too long to ever fit beside the title".to_string());
        let rows = plain(&transmission_runs(&data, TOAST_WIDTH));
        assert!(!rows[0].contains("far too long"), "{:?}", rows[0]);
        assert!(rows[0].contains("TRANSMISSION"), "{:?}", rows[0]);
    }

    #[test]
    fn title_row_marks_the_label_runs() {
        let rows = transmission_runs(&data(), TOAST_WIDTH);
        let title = &rows[0];
        assert!(title.iter().any(|r| r.text == "●" && r.token == "acc"));
        assert!(title
            .iter()
            .any(|r| r.text == " TRANSMISSION" && r.token == "ink" && r.bold));
        assert!(title
            .iter()
            .any(|r| r.text.contains("undo") && r.token == "dim"));
    }

    #[test]
    fn too_narrow_is_no_toast() {
        assert!(transmission_runs(&data(), 11).is_empty());
    }
}
