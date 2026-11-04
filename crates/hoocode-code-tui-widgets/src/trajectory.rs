//! A turn in the transcript (declutter spec "Transcript"): the header that
//! opens it (the user message) and the end line that closes a failed or
//! stopped one, plus the row helpers the tool rows share.
//!
//! Peek and full draw the header as the user message band; the turn's tool
//! rows, thinking and prose follow as their own components. Outline draws the
//! whole turn from the header (`outline::turn_rows`): the user message, the
//! recap rows, the answer and the end line. A turn that finished has no end
//! line; its result is in the status line.

use std::cell::RefCell;
use std::rc::Rc;

use hoocode_tui_ratatui::{Palette, RowCache};
use hoocode_tui_render::{fresh_stamp, mix_stamps, Component};

use crate::outline::{FlightStats, TurnEnd};
use crate::tool_output_view::ToolOutputView;

/// Trailing padding of a width-padded block line, ignoring the styling:
/// spaces go, and the SGR resets around them are re-attached after, so the
/// line still resets its colours. Byte-safe: the scan only walks ASCII
/// (`m`, digits, `;`, `\x1b[`).
pub(crate) fn trim_end_styled(line: &str) -> String {
    use hoocode_tui_util::text_slice as ts;
    let mut s = line.trim_end_matches(' ');
    let mut tail = String::new();
    loop {
        let bytes = s.as_bytes();
        if !bytes.ends_with(b"m") {
            break;
        }
        let mut i = bytes.len() - 1;
        while i > 0 && (bytes[i - 1].is_ascii_digit() || bytes[i - 1] == b';') {
            i -= 1;
        }
        if !(i >= 2 && bytes[i - 1] == b'[' && bytes[i - 2] == 0x1b) {
            break;
        }
        let esc = i - 2;
        tail = format!("{}{tail}", ts::suffix_from(s, esc));
        s = ts::range(s, 0, esc);
        s = s.trim_end_matches(' ');
    }
    format!("{s}{tail}")
}

/// Added and removed lines in a unified diff: `+` and `-` first-column
/// lines, skipping the `+++` / `---` file headers.
pub fn diff_stats(diff: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }
    (added, removed)
}

/// The end line of a turn in peek and full: a blank row, then ` ⊗ failed ·
/// 5.0s` or ` ⊗ stopped · 2.0s` (`outline::turn_end_rows`). It reads the
/// turn's shared outline data, so peek and outline always agree; a turn that
/// finished draws nothing.
#[derive(Debug)]
pub struct LandingLine {
    flight: Rc<RefCell<FlightStats>>,
    rev: u64,
}

impl LandingLine {
    pub fn new(flight: Rc<RefCell<FlightStats>>) -> Self {
        Self {
            flight,
            rev: fresh_stamp(),
        }
    }

    /// The end the row shows: `None` for a turn that finished.
    pub fn end(&self) -> Option<TurnEnd> {
        crate::outline::turn_end(&self.flight.borrow())
    }
}

impl Component for LandingLine {
    fn render_stamp(&self) -> Option<u64> {
        let key = self.flight.try_borrow().ok()?.render_key()?;
        Some(mix_stamps([self.rev, key]))
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        let palette = Palette::current();
        crate::outline::turn_end_rows(&self.flight.borrow(), &palette, width as usize)
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

/// The header opening a turn: the user message (` › prompt` on the surface
/// colour, wrapped, never cut). In outline it draws the turn's whole block
/// instead (`outline::turn_rows`), with one blank row above every turn but
/// the first.
#[derive(Debug)]
pub struct MissionHeader {
    seq: usize,
    prompt: String,
    view: ToolOutputView,
    flight: Option<Rc<RefCell<FlightStats>>>,
    cache: RefCell<RowCache>,
    rev: u64,
}

impl MissionHeader {
    pub fn new(seq: usize, prompt: &str) -> Self {
        Self {
            seq,
            prompt: prompt.to_string(),
            view: ToolOutputView::Peek,
            flight: None,
            cache: RefCell::new(RowCache::default()),
            rev: fresh_stamp(),
        }
    }

    /// The view dial's current stop: peek and full draw the user message,
    /// outline the turn's whole block.
    pub fn set_view(&mut self, view: ToolOutputView) {
        self.view = view;
        self.rev = fresh_stamp();
        self.cache.borrow_mut().clear();
    }

    /// The turn's outline data, refreshed by the transcript as the turn runs
    /// and ends; the header reads it on every outline render.
    pub fn set_flight(&mut self, flight: Rc<RefCell<FlightStats>>) {
        self.flight = Some(flight);
        self.rev = fresh_stamp();
    }
}

impl Component for MissionHeader {
    /// Peek and full draw what the header was built with. Outline reads the
    /// shared data too, so its key goes in (none while the turn runs).
    fn render_stamp(&self) -> Option<u64> {
        if self.view != ToolOutputView::Outline {
            return Some(self.rev);
        }
        let Some(flight) = &self.flight else {
            return Some(self.rev);
        };
        let key = flight.try_borrow().ok()?.render_key()?;
        Some(mix_stamps([self.rev, key]))
    }

    fn render(&mut self, width: u16) -> Vec<String> {
        if width == 0 {
            return Vec::new();
        }
        let palette = Palette::current();
        if self.view == ToolOutputView::Outline {
            if let Some(flight) = &self.flight {
                let mut rows = Vec::new();
                if self.seq > 1 {
                    rows.push(String::new());
                }
                rows.extend(crate::outline::turn_rows(
                    &self.prompt,
                    &flight.borrow(),
                    &palette,
                    width as usize,
                ));
                return rows;
            }
        }
        let theme = hoocode_tui_ratatui::current_theme_key();
        if let Some(cached) = self.cache.borrow().get(width, &theme) {
            return cached.to_vec();
        }
        let lines = crate::user_band_rows(&self.prompt, width as usize, &palette);
        self.cache.borrow_mut().set(width, theme, lines.clone());
        lines
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoocode_tui_util::{strip_vt_control_characters, visible_width};

    #[test]
    fn diff_stats_skip_file_headers() {
        assert_eq!(diff_stats("--- a\n+++ b\n-old\n+new\n ctx\n"), (1, 1));
        assert_eq!(diff_stats(""), (0, 0));
    }

    #[test]
    fn the_header_is_the_user_message_band() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut header = MissionHeader::new(1, "check wrap.rs");
        let lines = header.render(100);
        let bare = strip_vt_control_characters(&lines.join("\n"));
        assert!(bare.starts_with(" › check wrap.rs"), "{bare}");
        assert!(!bare.contains("M1"), "{bare}");
        assert_eq!(visible_width(&lines[0]), 100);
    }

    #[test]
    fn the_header_wraps_long_prompts_under_the_text() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let prompt = "fix the overflow bug in the wrap helper so long prompts never get cut";
        let mut header = MissionHeader::new(1, prompt);
        let lines = header.render(40);
        let bare: Vec<String> = lines
            .iter()
            .map(|l| strip_vt_control_characters(l))
            .collect();
        assert!(bare.len() >= 2, "{bare:?}");
        assert!(bare[0].starts_with(" › "), "{bare:?}");
        assert!(bare[1].starts_with("   ") && !bare[1].starts_with("    "));
        assert!(!bare.join(" ").contains('…'), "{bare:?}");
        for line in &lines {
            assert_eq!(visible_width(line), 40);
        }
    }

    #[test]
    fn the_header_keeps_every_prompt_line() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut header = MissionHeader::new(2, "first half of the turn\nsecond half of it");
        let bare = strip_vt_control_characters(&header.render(80).join("\n"));
        assert!(bare.contains(" › first half of the turn"), "{bare}");
        assert!(bare.contains("   second half of it"), "{bare}");
    }
}
