//! The Mission Control card chrome (M6): the heavy warn frame every card
//! draws when it replaces the comms frame (spec §3.3, §4 c1–c2, c8). The top
//! border holds the title and a right label; the body rows sit between `┃`
//! edges; the bottom border holds the foot. Titles and edges read the
//! `warning` token, so `/theme` and NO_COLOR keep working (shapes carry the
//! state with no colour). Callers style the body and foot themselves.

use hoocode_code_tui_theme::theme;
use hoocode_tui_util::{truncate_to_width, visible_width};

/// The card's title and right label, already joined (`◉ …` plus `· step NN`
/// when the host knows the held step).
pub fn card_title(title: &str) -> String {
    let t = theme();
    t.fg("warning", &t.bold(title))
}

/// The warn frame edge for `text`.
fn edge(text: &str) -> String {
    theme().fg("warning", text)
}

/// `runs` cut to `budget` cells, ending in `…` when cut.
fn cut_runs(runs: &[String], budget: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut used = 0;
    for run in runs {
        let width = visible_width(run);
        if used + width <= budget {
            used += width;
            out.push(run.clone());
            continue;
        }
        let room = budget.saturating_sub(used);
        if room > 0 {
            out.push(truncate_to_width(run, room, "…", false));
        }
        break;
    }
    out
}

/// One edge row: `┏━ left ━…━ right ━┓`, exactly `width` cells: 2 corners,
/// 2 bars and 4 spaces are fixed (8), the rest is the fill. The title goes
/// first when the row is short; past that both sides are cut.
fn edge_row(corners: (char, char), left: &[String], right: &[String], width: usize) -> String {
    let budget = width.saturating_sub(9);
    let right = cut_runs(right, budget);
    let used_right: usize = right.iter().map(|run| visible_width(run)).sum();
    let left = cut_runs(left, budget.saturating_sub(used_right));
    let used_left: usize = left.iter().map(|run| visible_width(run)).sum();
    let fill = width.saturating_sub(8 + used_left + used_right).max(1);
    // Corners, two bars and four spaces are fixed (8); the fill is at
    // least one cell, like the simulator's `box`.
    format!(
        "{}{} {} {} {} {}{}",
        edge(&corners.0.to_string()),
        edge("━"),
        left.concat(),
        edge(&"━".repeat(fill)),
        right.concat(),
        edge("━"),
        edge(&corners.1.to_string()),
    )
}

/// Draws the card at `width` columns: the warn top edge with `title` and
/// `right`, `body` between `┃` edges, and the foot edge. Rows that do not
/// fit are cut with `…`, the title first. Below four columns the frame has
/// no room, so the body draws bare.
pub fn render_card(
    title: &str,
    right: &str,
    body: &[String],
    foot: &str,
    width: usize,
) -> Vec<String> {
    if width < 4 {
        return body
            .iter()
            .map(|line| truncate_to_width(line, width, "…", false))
            .collect();
    }
    let mut out = Vec::with_capacity(body.len() + 2);
    out.push(edge_row(
        ('┏', '┓'),
        &[card_title(title)],
        &[theme().fg("warning", right)],
        width,
    ));
    let inner = width.saturating_sub(2);
    for line in body {
        let cut = truncate_to_width(line, inner, "…", false);
        let pad = inner.saturating_sub(visible_width(&cut));
        let padded = format!("{cut}{}", " ".repeat(pad));
        out.push(format!("{}{}{}", edge("┃"), padded, edge("┃")));
    }
    out.push(edge_row(('┗', '┛'), &[theme().fg("dim", foot)], &[], width));
    out
}
