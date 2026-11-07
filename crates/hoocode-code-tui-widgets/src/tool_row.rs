//! The tool row of the declutter transcript (spec "Transcript"):
//! ` ● Name subject  facts`, then an indented dim body.
//!
//! One glyph, the tool name in bold, the subject in plain text, then two
//! spaces and the facts in dim, directly after the subject (never
//! right-aligned). No step number, no rail. Rows are drawn from spans through
//! a ratatui buffer, like every other row of the widgets.

use hoocode_tui_glyphs::status;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{ansi_spans, bold, row, spans_width, Palette};
use hoocode_tui_util::{strip_vt_control_characters, wrap_text_with_ansi};

/// Columns a body row sits in: past the glyph, under the tool name's column.
pub const BODY_INDENT: usize = 4;

/// Columns the calls of a subagent sit in under its row.
pub const SUB_INDENT: usize = 3;

/// What a tool row's glyph says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Done: green `●`.
    Done,
    /// Still running: accent `◉` (single view, decision 4/9).
    Running,
    /// Failed: red `⊗`.
    Failed,
    /// Stopped or denied: dim `⊗`.
    Stopped,
}

impl Mark {
    /// The glyph, styled for the palette.
    pub fn span(self, palette: &Palette) -> Span<'static> {
        match self {
            Mark::Done => Span::styled(status::DONE_STR.to_string(), palette.ok()),
            Mark::Running => Span::styled(status::RUNNING_STR.to_string(), palette.acc()),
            Mark::Failed => Span::styled(status::FAILED_STR.to_string(), palette.err()),
            Mark::Stopped => Span::styled(status::FAILED_STR.to_string(), palette.dim()),
        }
    }
}

/// A fact in dim: `30 lines`.
pub fn dim_fact(text: &str, palette: &Palette) -> Span<'static> {
    Span::styled(text.to_string(), palette.dim())
}

/// Facts joined by a dim ` · `. Empty facts are dropped, so a zero value
/// never shows.
pub fn join_facts(facts: Vec<Vec<Span<'static>>>, palette: &Palette) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for fact in facts.into_iter().filter(|f| !f.is_empty()) {
        if !out.is_empty() {
            out.push(Span::styled(" · ".to_string(), palette.dim()));
        }
        out.extend(fact);
    }
    out
}

/// `+2 −2`: added in green, removed in red; a zero count is left out.
pub fn diff_facts(added: usize, removed: usize, palette: &Palette) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    if added > 0 {
        out.push(Span::styled(format!("+{added}"), palette.ok()));
    }
    if removed > 0 {
        if !out.is_empty() {
            out.push(Span::raw(" ".to_string()));
        }
        out.push(Span::styled(format!("−{removed}"), palette.err()));
    }
    out
}

/// `N lines` / `1 line`; empty for zero.
pub fn lines_fact(total: usize) -> String {
    match total {
        0 => String::new(),
        1 => "1 line".to_string(),
        n => format!("{n} lines"),
    }
}

/// A short duration: `4s`, `1m12s`.
pub fn short_dur(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{}s", secs / 60, secs % 60)
    }
}

// ---- Structured header + cut order (UI-21 P3 header slice) ----
//
// One line: mark, bold tool, target, two spaces, facts. Shell fact order is
// `exit · size · duration`: the exit is primary, size then duration are
// secondary (dropped right-to-left, so the duration goes first).
//
// Cut order for a header that is still too wide: (1) secondary facts drop
// whole, right to left; (2) the target shortens to its minimum, a path
// dropping whole leading directories and keeping the file name, a command
// cutting at a word; (3) a failed row's reason cuts at a word, keeping 12+
// columns; (4) otherwise the reason drops and the target takes the space
// back. Primary facts never drop.
//
// Matches the `width_single_view` executable contract (`fit_header`) on the
// `agent/u21-width` branch; no deviation.

/// Grid cells of a header string: fixtures are ASCII except `…`, one cell each.
fn header_cells(s: &str) -> usize {
    s.chars().count()
}

/// Whole leading words of `text` that fit in `avail` cells.
fn keep_words(text: &str, avail: usize) -> String {
    let mut kept = String::new();
    for w in text.split_whitespace() {
        let cand = if kept.is_empty() {
            w.to_string()
        } else {
            format!("{kept} {w}")
        };
        if header_cells(&cand) > avail {
            break;
        }
        kept = cand;
    }
    kept
}

/// Smallest whole-word command prefix at 12+ cells, then `…`: the target
/// minimum for a command.
fn min_command(cmd: &str) -> String {
    let mut kept = String::new();
    for w in cmd.split_whitespace() {
        kept = if kept.is_empty() {
            w.to_string()
        } else {
            format!("{kept} {w}")
        };
        if header_cells(&kept) >= 12 {
            break;
        }
    }
    format!("{kept}…")
}

/// A file name longer than the row cuts in the middle, keeping the extension.
fn middle_cut(name: &str, avail: usize) -> String {
    if header_cells(name) <= avail {
        return name.to_string();
    }
    if avail == 0 {
        return String::new();
    }
    if avail == 1 {
        return "…".to_string();
    }
    let tail = (avail - 1) / 2;
    let head = avail - 1 - tail;
    let chars: Vec<char> = name.chars().collect();
    let h: String = chars.iter().take(head).collect();
    let t: String = chars[chars.len() - tail..].iter().collect();
    format!("{h}…{t}")
}

/// A path shortens through whole leading directories first, keeping the file name.
fn shorten_path(path: &str, avail: usize) -> String {
    if header_cells(path) <= avail {
        return path.to_string();
    }
    let mut segs: Vec<&str> = path.split('/').collect();
    while segs.len() > 1 {
        segs.remove(0);
        let cand = format!("…/{}", segs.join("/"));
        if header_cells(&cand) <= avail {
            return cand;
        }
    }
    middle_cut(segs.last().copied().unwrap_or(path), avail)
}

/// A command cuts at a word, never below [`min_command`].
fn shorten_command(cmd: &str, avail: usize) -> String {
    if header_cells(cmd) <= avail {
        return cmd.to_string();
    }
    let kept = keep_words(cmd, avail.saturating_sub(1));
    if header_cells(&kept) >= 12 {
        return format!("{kept}…");
    }
    min_command(cmd)
}

/// A target shortens to its minimum, never dropping: paths drop leading
/// directories, commands cut at a word.
fn shorten_target(target: &str, avail: usize) -> String {
    if header_cells(target) <= avail {
        return target.to_string();
    }
    if target.contains('/') {
        shorten_path(target, avail)
    } else {
        shorten_command(target, avail)
    }
}

/// A failed row's reason cut at a word, or dropped when 12 columns do not fit.
fn cut_reason(reason: &str, avail: usize) -> Option<String> {
    if header_cells(reason) <= avail {
        return Some(reason.to_string());
    }
    if avail < 12 {
        return None;
    }
    let kept = keep_words(reason, avail - 1);
    if kept.is_empty() {
        return None;
    }
    Some(format!("{kept}…"))
}

/// Plain header text through the cut order: `{mark} {tool} {target}  {facts}`
/// with ` · ` between the primary facts, the reason and the kept secondary
/// facts. Fact order is primary, reason, secondary, so Shell reads
/// `exit · size · duration` with the reason after the exit.
fn render_header_text(
    head: &str,
    target: &str,
    prim: &str,
    secs: &[&str],
    reason: Option<&str>,
) -> String {
    let mut out = format!("{head}{target}");
    let mut facts: Vec<&str> = Vec::new();
    if !prim.is_empty() {
        facts.push(prim);
    }
    if let Some(r) = reason {
        facts.push(r);
    }
    facts.extend(secs.iter().copied());
    if !facts.is_empty() {
        out.push_str("  ");
        out.push_str(&facts.join(" · "));
    }
    out
}

/// One single-view header line through the cut order. Primary facts never
/// drop; secondary facts drop whole right-to-left, then the target shortens,
/// then the reason cuts at 12+ columns or drops.
pub fn fit_header(
    mark: char,
    tool: &str,
    target: &str,
    primary: &[&str],
    secondary: &[&str],
    reason: Option<&str>,
    width: usize,
) -> String {
    let head = format!("{mark} {tool} ");
    let prim = primary.join(" · ");
    // Step 1: secondary facts drop whole, right to left.
    let mut kept: Vec<&str> = secondary.to_vec();
    loop {
        if header_cells(&render_header_text(&head, target, &prim, &kept, reason)) <= width {
            return render_header_text(&head, target, &prim, &kept, reason);
        }
        if kept.pop().is_none() {
            break;
        }
    }
    // Step 2: the target shortens against the full reason, never below its
    // minimum.
    let rpart = reason.map(|r| header_cells(r) + 3).unwrap_or(0);
    let avail_t = width.saturating_sub(header_cells(&head) + 2 + header_cells(&prim) + rpart);
    let short = shorten_target(target, avail_t);
    // Steps 3-4: the reason cuts at a word, or drops.
    let base = render_header_text(&head, &short, &prim, &[], None);
    let avail_r = width.saturating_sub(header_cells(&base) + 3);
    match reason {
        Some(r) => match cut_reason(r, avail_r) {
            Some(cut) => format!("{base} · {cut}"),
            None => base,
        },
        None => base,
    }
}

/// The glyph of `mark` as a one-cell char, for measuring a styled header.
fn mark_char(mark: Mark) -> char {
    match mark {
        Mark::Done | Mark::Running => '●',
        Mark::Failed | Mark::Stopped => '⊗',
    }
}

/// One structured header line as spans: the mark, the tool in bold, the
/// (possibly shortened) target in plain text, two spaces, then the facts -
/// the exit red on a failed row, everything else dim. Runs the same cut
/// order as [`fit_header`]: secondary drops right-to-left, the target
/// shortens, the reason cuts at 12+ columns or drops, primary never drops.
#[allow(clippy::too_many_arguments)]
pub fn header_spans(
    mark: Mark,
    tool: &str,
    target: &str,
    primary: &[&str],
    secondary: &[&str],
    reason: Option<&str>,
    width: usize,
    palette: &Palette,
) -> Vec<Span<'static>> {
    let head = format!("{} {tool} ", mark_char(mark));
    let prim = primary.join(" · ");
    // Step 1: secondary facts drop whole, right to left.
    let mut kept: Vec<&str> = secondary.to_vec();
    loop {
        if header_cells(&render_header_text(&head, target, &prim, &kept, reason)) <= width {
            break;
        }
        if kept.pop().is_none() {
            break;
        }
    }
    let full = render_header_text(&head, target, &prim, &kept, reason);
    let (short_target, kept_secs, cut_r) = if header_cells(&full) <= width {
        (target.to_string(), kept.clone(), reason.map(str::to_string))
    } else {
        // Step 2: the target shortens against the full reason.
        let rpart = reason.map(|r| header_cells(r) + 3).unwrap_or(0);
        let avail_t = width.saturating_sub(header_cells(&head) + 2 + header_cells(&prim) + rpart);
        let short = shorten_target(target, avail_t);
        // Steps 3-4: the reason cuts at a word, or drops.
        let base = render_header_text(&head, &short, &prim, &[], None);
        let avail_r = width.saturating_sub(header_cells(&base) + 3);
        let cut = match reason {
            Some(r) => cut_reason(r, avail_r),
            None => None,
        };
        (short, Vec::new(), cut)
    };
    let mut spans = vec![
        mark.span(palette),
        Span::raw(" ".to_string()),
        Span::styled(tool.to_string(), bold(palette.ink())),
        Span::raw(" ".to_string()),
        Span::styled(short_target, palette.ink()),
    ];
    let failed_exit = matches!(mark, Mark::Failed);
    let mut first = true;
    let mut push_fact = |text: &str, err: bool, spans: &mut Vec<Span<'static>>| {
        if first {
            spans.push(Span::raw("  ".to_string()));
            first = false;
        } else {
            spans.push(Span::styled(" · ".to_string(), palette.dim()));
        }
        if err {
            spans.push(Span::styled(text.to_string(), palette.err()));
        } else {
            spans.push(Span::styled(text.to_string(), palette.dim()));
        }
    };
    for fact in primary.iter().filter(|f| !f.is_empty()) {
        let err = failed_exit && fact.starts_with("exit ");
        push_fact(fact, err, &mut spans);
    }
    if let Some(r) = cut_r.as_deref() {
        push_fact(r, false, &mut spans);
    }
    for fact in kept_secs.iter().filter(|f| !f.is_empty()) {
        push_fact(fact, false, &mut spans);
    }
    spans
}

/// A tool row as rows: ` ● label  facts`. The label (bold name, subject)
/// arrives styled and wraps under itself rather than being cut; the facts
/// follow the last label row after two spaces, or take a row of their own
/// when they do not fit beside it. `indent` is the nesting (0 for a call,
/// [`SUB_INDENT`] for a call inside a subagent).
pub fn tool_rows(
    indent: usize,
    mark: Mark,
    label: &str,
    facts: Vec<Span<'static>>,
    width: usize,
    palette: &Palette,
) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let lead = if indent == 0 { 1 } else { indent };
    let avail = width.saturating_sub(lead + 2).max(1);
    let mut pieces = wrap_text_with_ansi(label, avail);
    if pieces.is_empty() {
        pieces.push(String::new());
    }
    let facts_width = spans_width(&facts);
    let count = pieces.len();
    let hang = " ".repeat(lead + 2);
    let mut rows = Vec::with_capacity(count + 1);
    for (index, piece) in pieces.into_iter().enumerate() {
        let mut spans = if index == 0 {
            vec![
                Span::raw(" ".repeat(lead)),
                mark.span(palette),
                Span::raw(" ".to_string()),
            ]
        } else {
            vec![Span::raw(hang.clone())]
        };
        spans.extend(ansi_spans(&piece));
        let last = index + 1 == count;
        if last && facts_width > 0 && spans_width(&spans) + 2 + facts_width <= width {
            spans.push(Span::raw("  ".to_string()));
            spans.extend(facts.clone());
            rows.push(row(spans, width));
        } else {
            rows.push(row(spans, width));
            if last && facts_width > 0 {
                let mut own = vec![Span::raw(hang.clone())];
                own.extend(facts.clone());
                rows.push(row(own, width));
            }
        }
    }
    rows
}

/// The label of a call line: the tool name in bold, one space, the subject
/// in plain text. The `name` is the first word of `plain`'s first line.
pub fn label_spans(plain: &str, palette: &Palette) -> Vec<Span<'static>> {
    let plain = plain.trim();
    let (name, rest) = match plain.split_once(char::is_whitespace) {
        Some((name, rest)) => (name, rest.trim_start()),
        None => (plain, ""),
    };
    let mut spans = vec![Span::styled(name.to_string(), bold(palette.ink()))];
    if !rest.is_empty() {
        spans.push(Span::styled(format!(" {rest}"), palette.ink()));
    }
    spans
}

/// A dim body row from a plain line, at the body indent.
pub fn dim_body_row(text: &str, width: usize, palette: &Palette) -> String {
    row(
        vec![
            Span::raw(" ".repeat(BODY_INDENT)),
            Span::styled(text.to_string(), palette.dim()),
        ],
        width,
    )
}

/// The hint row of a fold: `    … 27 more lines`, dim.
pub fn fold_row(text: &str, width: usize, palette: &Palette) -> String {
    dim_body_row(text, width, palette)
}

/// Whether a rendered row is blank once its styling is gone.
pub fn is_blank_row(line: &str) -> bool {
    strip_vt_control_characters(line).trim().is_empty()
}
