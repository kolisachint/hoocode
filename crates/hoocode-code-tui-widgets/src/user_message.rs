//! The user message (declutter spec "Transcript"): full-width rows on the
//! `surface` colour, ` › ` in teal, the text in normal weight. Wrapped and
//! further lines indent 3, under the text. No frame, no bold, no title.

use hoocode_tui_ratatui::core::style::Style;
use hoocode_tui_ratatui::core::text::Span;
use hoocode_tui_ratatui::{row, spans_width, Palette};
use hoocode_tui_render::Component;
use hoocode_tui_util::wrap_text_with_ansi;

/// Columns before the text: a blank, the `›` marker and a blank.
pub const USER_INDENT: usize = 3;

/// The marker in front of a user message's first row.
const MARKER: &str = " › ";

/// A user message's rows: ` › text` on the surface colour, every row the full
/// width. The text wraps at spaces with one blank column kept at the right
/// edge; continuation rows and later lines start at column 3. A blank line in
/// the text stays a blank surface row. With colour off the surface is gone and
/// the `›` still marks the text.
pub fn user_band_rows(text: &str, width: usize, palette: &Palette) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let bg = palette.bg_color("surface");
    let on_surface = |style: Style| match bg {
        Some(bg) => style.bg(bg),
        None => style,
    };
    let room = width.saturating_sub(USER_INDENT + 1).max(1);
    let mut pieces: Vec<String> = Vec::new();
    for line in text.trim_end().split('\n') {
        let line = line.trim_end();
        if line.trim().is_empty() {
            pieces.push(String::new());
            continue;
        }
        pieces.extend(wrap_text_with_ansi(line, room));
    }
    if pieces.is_empty() {
        pieces.push(String::new());
    }
    pieces
        .into_iter()
        .enumerate()
        .map(|(index, piece)| {
            let lead = if index == 0 {
                Span::styled(MARKER.to_string(), on_surface(palette.acc()))
            } else {
                Span::styled(" ".repeat(USER_INDENT), on_surface(Style::default()))
            };
            let mut spans = vec![lead, Span::styled(piece, on_surface(palette.ink()))];
            let pad = width.saturating_sub(spans_width(&spans));
            spans.push(Span::styled(" ".repeat(pad), on_surface(Style::default())));
            row(spans, width)
        })
        .collect()
}

/// A user message as a transcript item: its band rows. The app draws the
/// same rows through the turn's header (`trajectory::MissionHeader`).
pub struct UserMessageComponent {
    text: String,
}

impl UserMessageComponent {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_string(),
        }
    }
}

impl Component for UserMessageComponent {
    fn render(&mut self, width: u16) -> Vec<String> {
        let palette = Palette::current();
        user_band_rows(&self.text, width as usize, &palette)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}
