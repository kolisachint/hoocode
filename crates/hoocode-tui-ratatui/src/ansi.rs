//! ANSI text as ratatui spans: the bridge that lets Mission Control
//! surfaces draw through a ratatui [`Buffer`] while block bodies still
//! arrive as chalk-styled strings.
//!
//! [`ansi_spans`] parses the SGR sequences chalk emits (truecolor or 256
//! colours, `bold`/`italic`/`underline`/`inverse`/`strikethrough`) into
//! styled spans; every other escape (OSC hyperlinks, APC, non-SGR CSI) is
//! skipped as zero-width, exactly the way
//! [`hoocode_tui_util::strip_vt_control_characters`] treats it, so widths
//! stay in step with [`hoocode_tui_util::visible_width`].
//!
//! Known gap (accepted): an OSC-8 hyperlink's text survives but its link
//! does not, so links inside trajectory lines are no longer clickable.
//! Scanning goes
//! through [`hoocode_tui_util::extract_ansi_code`], the same splitter the
//! width functions use, so a lone or unterminated ESC stays text here too.
//!
//! [`truncate_spans`] cuts spans to a cell budget with the same cells
//! [`hoocode_tui_util::truncate_to_width`] keeps: the kept cell prefix,
//! then the ellipsis in the default style.

use hoocode_tui_util::{extract_ansi_code, truncate_to_width, visible_width};
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::text::Span;

/// The SGR attribute state while scanning one line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SgrState {
    fg: Option<Color>,
    bg: Option<Color>,
    mods: Modifier,
}

impl SgrState {
    fn style(&self) -> Style {
        let mut style = Style::default();
        if let Some(fg) = self.fg {
            style = style.fg(fg);
        }
        if let Some(bg) = self.bg {
            style = style.bg(bg);
        }
        if !self.mods.is_empty() {
            style = style.add_modifier(self.mods);
        }
        style
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// `38;5;n` / `48;5;n`.
fn indexed(parts: &[&str], i: usize) -> Option<Color> {
    parts
        .get(i + 2)
        .and_then(|n| n.parse::<u8>().ok())
        .map(Color::Indexed)
}

/// `38;2;r;g;b` / `48;2;r;g;b`.
fn rgb(parts: &[&str], i: usize) -> Option<Color> {
    let (r, g, b) = (
        parts.get(i + 2)?.parse::<u8>().ok()?,
        parts.get(i + 3)?.parse::<u8>().ok()?,
        parts.get(i + 4)?.parse::<u8>().ok()?,
    );
    Some(Color::Rgb(r, g, b))
}

/// Feeds one SGR parameter string (between `ESC [` and `m`) into `state`,
/// mirroring `AnsiCodeTracker`: extended colours first, then single codes.
/// Unparseable parts are skipped one at a time.
fn apply_sgr(state: &mut SgrState, params: &str) {
    if params.is_empty() {
        state.reset();
        return;
    }
    let parts: Vec<&str> = params.split(';').collect();
    let mut i = 0;
    while i < parts.len() {
        let Ok(code) = parts[i].parse::<i64>() else {
            i += 1;
            continue;
        };
        if code == 38 || code == 48 {
            // An announced `;5` or `;2` colour with missing bytes aborts the
            // whole sequence: the orphaned `2` must not fall through and
            // read as DIM below.
            if parts.get(i + 1) == Some(&"5") || parts.get(i + 1) == Some(&"2") {
                let color = if parts.get(i + 1) == Some(&"5") {
                    indexed(&parts, i).map(|c| (c, 3))
                } else {
                    rgb(&parts, i).map(|c| (c, 5))
                };
                let Some((color, used)) = color else {
                    return;
                };
                if code == 38 {
                    state.fg = Some(color);
                } else {
                    state.bg = Some(color);
                }
                i += used;
                continue;
            }
        }
        match code {
            0 => state.reset(),
            1 => state.mods.insert(Modifier::BOLD),
            2 => state.mods.insert(Modifier::DIM),
            3 => state.mods.insert(Modifier::ITALIC),
            4 => state.mods.insert(Modifier::UNDERLINED),
            7 => state.mods.insert(Modifier::REVERSED),
            9 => state.mods.insert(Modifier::CROSSED_OUT),
            // Blink (5) and hidden (8) have no ratatui modifier; chalk never
            // emits them, so they are ignored rather than mis-styled.
            21 => state.mods.remove(Modifier::BOLD),
            22 => state.mods.remove(Modifier::BOLD | Modifier::DIM),
            23 => state.mods.remove(Modifier::ITALIC),
            24 => state.mods.remove(Modifier::UNDERLINED),
            27 => state.mods.remove(Modifier::REVERSED),
            29 => state.mods.remove(Modifier::CROSSED_OUT),
            39 => state.fg = None,
            49 => state.bg = None,
            30..=37 => state.fg = Some(Color::Indexed((code - 30) as u8)),
            90..=97 => state.fg = Some(Color::Indexed((code - 90 + 8) as u8)),
            40..=47 => state.bg = Some(Color::Indexed((code - 40) as u8)),
            100..=107 => state.bg = Some(Color::Indexed((code - 100 + 8) as u8)),
            _ => {}
        }
        i += 1;
    }
}

/// Pushes `text` with `style`, merging into the previous span when it carries
/// the same style. `text` holds no escapes: the scanner only passes it runs
/// between recognised sequences.
fn push_text(out: &mut Vec<Span<'static>>, style: Style, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut() {
        if last.style == style {
            last.content.to_mut().push_str(text);
            return;
        }
    }
    out.push(Span::styled(text.to_string(), style));
}

/// `text` as ratatui spans: SGR runs become span styles, everything else
/// passes through as text. Adjacent runs with the same style merge.
///
/// Every offset the scanner holds is a char boundary (the loop only stops
/// at ASCII ESC bytes and string ends), so the `get` slices below always
/// hit; a `None` ends the scan instead of panicking.
pub fn ansi_spans(text: &str) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut state = SgrState::default();
    let mut pos = 0;
    let mut run_start: Option<usize> = None;
    while pos < text.len() {
        let Some(rest) = text.get(pos..) else {
            break;
        };
        if rest.starts_with('\x1b') {
            if let Some((code, len)) = extract_ansi_code(text, pos) {
                // Flushes the pending text run with the style so far.
                if let Some(start) = run_start.take() {
                    if let Some(run) = text.get(start..pos) {
                        push_text(&mut out, state.style(), run);
                    }
                }
                if let Some(params) = code.strip_prefix("\x1b[").and_then(|s| s.strip_suffix('m')) {
                    apply_sgr(&mut state, params);
                }
                pos += len;
                continue;
            }
        }
        if run_start.is_none() {
            run_start = Some(pos);
        }
        pos += rest.chars().next().map_or(1, |c| c.len_utf8());
    }
    if let Some(start) = run_start.take() {
        if let Some(run) = text.get(start..text.len()) {
            push_text(&mut out, state.style(), run);
        }
    }
    out
}

/// Reroots unstyled spans onto `style`, keeping explicitly styled runs as
/// they are: what wrapping one chalk `fg` call around parsed text means.
/// Chalk never re-opens the outer style after an inner close, so neither
/// does this.
pub fn with_base_style(spans: Vec<Span<'static>>, style: Style) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|span| {
            if span.style == Style::default() {
                Span::styled(span.content.into_owned(), style)
            } else {
                span
            }
        })
        .collect()
}

/// The visible cells `spans` take.
pub fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|s| visible_width(s.content.as_ref()))
        .sum()
}

/// `spans` cut to `max_width` cells: the kept cell prefix, then `ellipsis`
/// in the default style. Fits return unchanged (no ellipsis). An empty
/// budget returns nothing. The cut matches [`truncate_to_width`] cell for
/// cell: whole spans are kept while they fit, and the span holding the cut
/// keeps its bare cell prefix (its style, no ellipsis room reserved twice).
pub fn truncate_spans(
    spans: Vec<Span<'static>>,
    max_width: usize,
    ellipsis: &str,
) -> Vec<Span<'static>> {
    if max_width == 0 {
        return Vec::new();
    }
    if spans_width(&spans) <= max_width {
        return spans;
    }
    let target = max_width.saturating_sub(visible_width(ellipsis));
    let mut out = Vec::new();
    let mut used = 0;
    for span in spans {
        let width = visible_width(span.content.as_ref());
        if used + width <= target {
            used += width;
            out.push(span);
            continue;
        }
        let remaining = target.saturating_sub(used);
        // The bare cell prefix: no ellipsis of its own, so the cut lands
        // exactly where `truncate_to_width` would cut the joined line.
        let prefix = if remaining == 0 {
            String::new()
        } else {
            truncate_to_width(span.content.as_ref(), remaining, "", false).replace("\x1b[0m", "")
        };
        if !prefix.is_empty() {
            out.push(Span::styled(prefix, span.style));
        }
        if !ellipsis.is_empty() {
            out.push(Span::styled(ellipsis.to_string(), Style::default()));
        }
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styled_text(spans: &[Span<'_>]) -> String {
        spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<Vec<_>>()
            .join("|")
    }

    #[test]
    fn plain_text_is_one_default_span() {
        let spans = ansi_spans("hello");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "hello");
        assert_eq!(spans[0].style, Style::default());
    }

    #[test]
    fn truecolor_sgr_becomes_rgb_spans() {
        let spans = ansi_spans("\x1b[38;2;255;92;122mhi\x1b[39m ok");
        assert_eq!(styled_text(&spans), "hi| ok");
        assert_eq!(
            spans[0].style,
            Style::default().fg(Color::Rgb(255, 92, 122))
        );
        assert_eq!(spans[1].style, Style::default());
    }

    #[test]
    fn modifiers_set_and_clear() {
        let spans = ansi_spans("\x1b[1mbold\x1b[22m plain \x1b[3mit\x1b[23m");
        assert_eq!(
            spans[0].style,
            Style::default().add_modifier(Modifier::BOLD)
        );
        assert_eq!(spans[1].style, Style::default());
        assert_eq!(
            spans[2].style,
            Style::default().add_modifier(Modifier::ITALIC)
        );
    }

    #[test]
    fn full_reset_clears_everything() {
        let spans = ansi_spans("\x1b[1;38;2;1;2;3mab\x1b[0mcd");
        assert_eq!(
            spans[0].style,
            Style::default()
                .fg(Color::Rgb(1, 2, 3))
                .add_modifier(Modifier::BOLD)
        );
        assert_eq!(spans[1].style, Style::default());
    }

    #[test]
    fn incomplete_extended_colours_abort_the_sequence() {
        // A truncated `38;2` must not leak its `2` as DIM, and a truncated
        // `38;5` must not style anything either.
        for seq in ["\x1b[38;2;255;92m", "\x1b[38;5m", "\x1b[48;2;1m"] {
            let spans = ansi_spans(&format!("{seq}hi"));
            assert_eq!(spans.len(), 1, "{seq:?}");
            assert_eq!(spans[0].content.as_ref(), "hi");
            assert_eq!(spans[0].style, Style::default(), "{seq:?}");
        }
    }

    #[test]
    fn backgrounds_and_256_colours_parse() {
        let spans = ansi_spans("\x1b[48;5;99mab\x1b[49m \x1b[38;5;196mc\x1b[0m");
        assert_eq!(spans[0].style.bg, Some(Color::Indexed(99)));
        assert_eq!(spans[2].style.fg, Some(Color::Indexed(196)));
    }

    #[test]
    fn non_sgr_escapes_are_skipped_as_zero_width() {
        // An OSC-8 hyperlink leaves its text, unstyled by the escape itself.
        let spans = ansi_spans("\x1b]8;;https://x\x07link\x1b]8;;\x07");
        assert_eq!(styled_text(&spans), "link");
        assert_eq!(spans_width(&spans), visible_width("link"));
    }

    #[test]
    fn lone_and_unterminated_escapes_stay_text() {
        assert_eq!(styled_text(&ansi_spans("a\x1b[31")), "a\x1b[31");
        assert_eq!(styled_text(&ansi_spans("\x1bx")), "\x1bx");
    }

    #[test]
    fn truncate_keeps_whole_spans_then_cuts_with_ellipsis() {
        let spans = ansi_spans("\x1b[1mhello\x1b[0m world");
        let cut = truncate_spans(spans, 8, "…");
        let text: String = cut.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "hello w…");
        assert_eq!(spans_width(&cut), 8);
        assert_eq!(cut[0].style, Style::default().add_modifier(Modifier::BOLD));
        assert_eq!(cut.last().unwrap().style, Style::default());
    }

    #[test]
    fn truncate_matching_truncate_to_width_on_styled_text() {
        let styled = "\x1b[38;2;255;92;122mhello brave\x1b[39m new \x1b[1mworld\x1b[22m end";
        for width in [1, 5, 10, 15, 20] {
            let spans = truncate_spans(ansi_spans(styled), width, "…");
            let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
            let plain = hoocode_tui_util::strip_vt_control_characters(&truncate_to_width(
                styled, width, "…", false,
            ));
            assert_eq!(text, plain, "width {width}");
            assert_eq!(spans_width(&spans), visible_width(&plain));
        }
    }

    #[test]
    fn truncate_fitting_spans_come_back_unchanged() {
        let spans = ansi_spans("\x1b[1mhi\x1b[0m");
        let kept = truncate_spans(spans.clone(), 10, "…");
        assert_eq!(kept.len(), spans.len());
    }

    #[test]
    fn truncate_to_zero_is_empty() {
        assert!(truncate_spans(ansi_spans("hi"), 0, "…").is_empty());
    }
}
