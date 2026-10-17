//! Buffer to ANSI lines: the bridge from ratatui's cell grid to the line-based
//! `Component` model.

use ratatui_core::buffer::{Buffer, CellWidth};
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style};

/// Paints one row of `text` in `style`: a 1-row buffer drawn with the
/// style, converted with [`buffer_to_lines`]. For joining styled runs into
/// a longer line without a full widget. `text` is one row with no trailing
/// spaces (trailing styled blanks are kept) and no newlines.
pub fn paint(style: Style, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    // Characters always cover at least as many columns as cells: wide
    // symbols still fit, and trailing blanks of a too-wide buffer are
    // trimmed by `buffer_to_lines` when they carry no style.
    let width = u16::try_from(text.chars().count())
        .unwrap_or(u16::MAX)
        .max(1);
    let mut buf = Buffer::empty(Rect::new(0, 0, width, 1));
    buf.set_string(0, 0, text, style);
    buffer_to_lines(&buf).pop().unwrap_or_default()
}

/// The look of one cell, reduced to what SGR can express.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Look {
    fg: Color,
    bg: Color,
    modifier: Modifier,
}

impl Look {
    const DEFAULT: Self = Self {
        fg: Color::Reset,
        bg: Color::Reset,
        modifier: Modifier::empty(),
    };

    fn is_default(self) -> bool {
        self == Self::DEFAULT
    }

    /// The SGR codes for this look, without the leading `0` reset.
    fn codes(self) -> Vec<String> {
        let mut codes = Vec::new();
        if let Some(code) = color_code(self.fg, 30) {
            codes.push(code);
        }
        if let Some(code) = color_code(self.bg, 40) {
            codes.push(code);
        }
        for (flag, code) in [
            (Modifier::BOLD, "1"),
            (Modifier::DIM, "2"),
            (Modifier::ITALIC, "3"),
            (Modifier::UNDERLINED, "4"),
            (Modifier::REVERSED, "7"),
            (Modifier::CROSSED_OUT, "9"),
        ] {
            if self.modifier.contains(flag) {
                codes.push(code.to_string());
            }
        }
        codes
    }
}

/// The SGR code for a colour: `base` selects foreground (30) or background (40).
/// `Reset` has no code; the leading reset in [`push_sgr`] clears it.
fn color_code(color: Color, base: u8) -> Option<String> {
    // Named colours: the eight normal ones at `base`, the bright ones at `base + 60`.
    let named = |offset: u8| Some((base + offset).to_string());
    match color {
        Color::Reset => None,
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => Some((base + 60).to_string()),
        Color::LightRed => Some((base + 61).to_string()),
        Color::LightGreen => Some((base + 62).to_string()),
        Color::LightYellow => Some((base + 63).to_string()),
        Color::LightBlue => Some((base + 64).to_string()),
        Color::LightMagenta => Some((base + 65).to_string()),
        Color::LightCyan => Some((base + 66).to_string()),
        Color::White => Some((base + 67).to_string()),
        Color::Rgb(r, g, b) => Some(format!("{};2;{r};{g};{b}", base + 8)),
        Color::Indexed(n) => Some(format!("{};5;{n}", base + 8)),
    }
}

/// Appends one SGR sequence that moves the terminal from the default style to `look`:
/// a reset, then the look's codes. Default looks get the bare reset.
fn push_sgr(out: &mut String, look: Look) {
    let mut codes = vec!["0".to_string()];
    codes.extend(look.codes());
    out.push_str("\x1b[");
    out.push_str(&codes.join(";"));
    out.push('m');
}

/// One row of the buffer as a line: symbols concatenated, SGR where the style changes,
/// trailing default-styled blanks dropped.
fn row_line(buf: &Buffer, y: u16) -> String {
    let area = *buf.area();
    let width = area.width;

    // Collect (symbol, look) per visible column. A wide symbol owns the columns it covers,
    // so the continuation columns after it are not emitted.
    let mut cells: Vec<(String, Look)> = Vec::with_capacity(usize::from(width));
    let mut x: u16 = 0;
    while x < width {
        let Some(cell) = buf.cell((area.x + x, area.y + y)) else {
            break;
        };
        let look = Look {
            fg: cell.fg,
            bg: cell.bg,
            modifier: cell.modifier,
        };
        let symbol_width = cell.cell_width();
        cells.push((cell.symbol().to_string(), look));
        // Columns after a wide symbol are covered by it (ratatui resets them to blank).
        x = x.saturating_add(symbol_width.max(1));
    }

    // Drop trailing blanks that carry no style.
    let keep = cells
        .iter()
        .rposition(|(symbol, look)| !(look.is_default() && (symbol == " " || symbol.is_empty())))
        .map_or(0, |last| last + 1);

    let mut out = String::new();
    let mut current = Look::DEFAULT;
    let mut styled = false;
    for (symbol, look) in cells.iter().take(keep) {
        if *look != current {
            push_sgr(&mut out, *look);
            current = *look;
            styled = true;
        }
        out.push_str(symbol);
    }
    if styled {
        out.push_str("\x1b[0m");
    }
    out
}

/// Converts a buffer into one line per row, with ANSI SGR for styled cells.
///
/// Rows follow the buffer's area top to bottom. Plain text produces no escape codes; a
/// line that set any style ends with `ESC[0m`. Wide characters are emitted once, and
/// trailing unstyled spaces are trimmed.
pub fn buffer_to_lines(buf: &Buffer) -> Vec<String> {
    let height = buf.area().height;
    (0..height).map(|y| row_line(buf, y)).collect()
}

#[cfg(test)]
mod tests {
    use ratatui_core::layout::Rect;
    use ratatui_core::style::Style;

    use super::*;

    fn buffer(width: u16, height: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, width, height))
    }

    #[test]
    fn paint_wraps_one_row_in_its_style() {
        let line = paint(Style::default().fg(Color::Rgb(255, 92, 122)), "┃");
        assert_eq!(line, "\x1b[0;38;2;255;92;122m┃\x1b[0m");
        assert_eq!(paint(Style::default(), ""), "");
    }

    #[test]
    fn plain_text_has_no_escapes() {
        let mut buf = buffer(10, 2);
        buf.set_string(0, 0, "hello", Style::default());
        buf.set_string(0, 1, "hi", Style::default());
        assert_eq!(buffer_to_lines(&buf), vec!["hello", "hi"]);
    }

    #[test]
    fn style_change_mid_line_emits_sgr_and_final_reset() {
        let mut buf = buffer(10, 1);
        buf.set_string(0, 0, "ab", Style::default().fg(Color::Red));
        buf.set_string(2, 0, "cd", Style::default());
        assert_eq!(
            buffer_to_lines(&buf),
            vec!["\x1b[0;31mab\x1b[0mcd\x1b[0m".to_string()]
        );
    }

    #[test]
    fn wide_chars_are_emitted_once() {
        let mut buf = buffer(8, 1);
        buf.set_string(0, 0, "日本", Style::default());
        assert_eq!(buffer_to_lines(&buf), vec!["日本"]);
    }

    #[test]
    fn trailing_default_spaces_are_trimmed() {
        let mut buf = buffer(12, 1);
        buf.set_string(0, 0, "abc   ", Style::default());
        assert_eq!(buffer_to_lines(&buf), vec!["abc"]);
    }

    #[test]
    fn trailing_styled_blanks_are_kept() {
        let mut buf = buffer(6, 1);
        buf.set_string(0, 0, "ab", Style::default());
        buf.set_string(2, 0, "  ", Style::default().bg(Color::Blue));
        assert_eq!(
            buffer_to_lines(&buf),
            vec!["ab\x1b[0;44m  \x1b[0m".to_string()]
        );
    }

    #[test]
    fn no_style_means_no_sgr() {
        let mut buf = buffer(5, 1);
        buf.set_string(0, 0, "x y", Style::default());
        let lines = buffer_to_lines(&buf);
        assert!(!lines[0].contains('\x1b'));
    }

    #[test]
    fn colours_and_modifiers_map_to_sgr() {
        let mut buf = buffer(4, 1);
        buf.set_string(
            0,
            0,
            "a",
            Style::default()
                .fg(Color::LightCyan)
                .bg(Color::Rgb(1, 2, 3))
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        );
        buf.set_string(1, 0, "b", Style::default().fg(Color::Indexed(200)));
        assert_eq!(
            buffer_to_lines(&buf),
            vec!["\x1b[0;96;48;2;1;2;3;1;7ma\x1b[0;38;5;200mb\x1b[0m".to_string()]
        );
    }
}
