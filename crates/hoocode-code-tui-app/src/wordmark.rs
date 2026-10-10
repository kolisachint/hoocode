//! The compact startup banner (`core/wordmark.ts`).

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How long each half of the header cursor's blink lasts: `_` shows for this
/// long, then a space for this long.
pub const CURSOR_BLINK_PERIOD: Duration = Duration::from_millis(530);

/// The header cursor's blink, driven by the app. Ghostty ignores SGR 5, so the
/// UI loop flips `visible` on its own clock; the header reads it on render.
#[derive(Debug)]
pub struct CursorBlink {
    visible: Rc<Cell<bool>>,
    /// When the next flip is due. `None` while the blink is not running.
    next_flip: Option<Instant>,
}

impl Default for CursorBlink {
    fn default() -> Self {
        Self::new()
    }
}

impl CursorBlink {
    /// Starts solid: the cursor shows `_` until the blink runs.
    pub fn new() -> Self {
        Self {
            visible: Rc::new(Cell::new(true)),
            next_flip: None,
        }
    }

    /// The shared flag the header's cursor reads when it renders.
    pub fn visible(&self) -> Rc<Cell<bool>> {
        self.visible.clone()
    }

    /// When the loop must wake for the next flip. `None` while not blinking.
    pub fn deadline(&self) -> Option<Instant> {
        self.next_flip
    }

    /// Advance the blink to `now`. `running` says whether the blink should run
    /// at all; when it stops, the cursor is left solid so the last frame reads
    /// as a plain `_`. Returns true when the visible state changed, so the
    /// header must repaint.
    pub fn advance(&mut self, now: Instant, running: bool) -> bool {
        if !running {
            self.next_flip = None;
            if self.visible.get() {
                return false;
            }
            self.visible.set(true);
            return true;
        }
        match self.next_flip {
            None => {
                // The first call starts the clock; nothing flips yet.
                self.next_flip = Some(now + CURSOR_BLINK_PERIOD);
                false
            }
            Some(due) if now >= due => {
                self.visible.set(!self.visible.get());
                self.next_flip = Some(now + CURSOR_BLINK_PERIOD);
                true
            }
            Some(_) => false,
        }
    }
}

/// The three-line owl glyph beside the brand text.
const WORDMARK_GLYPH: [&str; 3] = ["▟▀▀▀▀▀▙", "▌▟▙ ▟▙▐", "▜▄▄▄▄▄▛"];

const GLYPH_GAP: &str = "  ";

type Style<'a> = &'a dyn Fn(&str) -> String;

/// `CompactWordmarkOptions`.
pub struct CompactWordmarkOptions<'a> {
    pub app_name: &'a str,
    pub version: &'a str,
    pub cwd: &'a str,
    /// Tagline next to the version (default "coding agent").
    pub tagline: Option<&'a str>,
    /// The brand name.
    pub accent: Style<'a>,
    /// The owl glyph; defaults to `accent`.
    pub glyph: Option<Style<'a>>,
    /// Secondary text (tagline, version, cwd).
    pub dim: Style<'a>,
    /// Separators.
    pub muted: Style<'a>,
    /// The trailing blinking cursor, when set.
    pub cursor: Option<Style<'a>>,
}

/// `buildCompactWordmark`: glyph beside brand, tagline + version, and cwd,
/// flush against the left edge.
pub fn build_compact_wordmark(options: &CompactWordmarkOptions<'_>) -> String {
    let accent = options.accent;
    let glyph_style = options.glyph.unwrap_or(accent);
    let tagline = options.tagline.unwrap_or("coding agent");

    // Highlight the "hoo" prefix when present, otherwise accent the whole name.
    let name = match options.app_name.strip_prefix("hoo") {
        Some(rest) => format!("{}{}{rest}", accent("hoo"), (options.muted)("│")),
        None => accent(options.app_name),
    };
    let brand = match options.cursor {
        Some(cursor) => format!("{name}{}", cursor("_")),
        None => name,
    };
    let dim = options.dim;
    let right = [
        brand,
        format!(
            "{} {} {}",
            dim(tagline),
            (options.muted)("·"),
            dim(&format!("v{}", options.version))
        ),
        dim(options.cwd),
    ];
    WORDMARK_GLYPH
        .iter()
        .zip(right.iter())
        .map(|(glyph, text)| format!("{}{GLYPH_GAP}{text}", glyph_style(glyph)))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_starts_solid_with_no_deadline() {
        let blink = CursorBlink::new();
        assert!(blink.visible().get());
        assert_eq!(blink.deadline(), None);
    }

    #[test]
    fn cursor_flips_every_period_while_running() {
        let mut blink = CursorBlink::new();
        let start = Instant::now();
        // The first run only starts the clock.
        assert!(!blink.advance(start, true));
        assert_eq!(blink.deadline(), Some(start + CURSOR_BLINK_PERIOD));
        // Before the deadline nothing changes.
        assert!(!blink.advance(start + Duration::from_millis(529), true));
        assert!(blink.visible().get());
        // At the deadline it turns to a space, and the next flip is a period later.
        assert!(blink.advance(start + CURSOR_BLINK_PERIOD, true));
        assert!(!blink.visible().get());
        let second = start + CURSOR_BLINK_PERIOD * 2;
        assert_eq!(blink.deadline(), Some(second));
        assert!(blink.advance(second, true));
        assert!(blink.visible().get());
    }

    #[test]
    fn stopping_leaves_the_cursor_solid_and_drops_the_deadline() {
        let mut blink = CursorBlink::new();
        let start = Instant::now();
        blink.advance(start, true);
        blink.advance(start + CURSOR_BLINK_PERIOD, true);
        assert!(!blink.visible().get(), "blink is mid-cycle on a space");
        // Stopping repaints once, to show the solid `_` again.
        assert!(blink.advance(start + CURSOR_BLINK_PERIOD, false));
        assert!(blink.visible().get());
        assert_eq!(blink.deadline(), None);
        // Stopped and already solid: nothing to repaint, no wakeups.
        assert!(!blink.advance(start + CURSOR_BLINK_PERIOD * 5, false));
        assert_eq!(blink.deadline(), None);
    }

    #[test]
    fn resuming_after_a_pause_waits_a_full_period() {
        let mut blink = CursorBlink::new();
        let start = Instant::now();
        blink.advance(start, true);
        blink.advance(start, false);
        // A pause must not leave a stale deadline that flips at once.
        let later = start + Duration::from_secs(10);
        assert!(!blink.advance(later, true));
        assert_eq!(blink.deadline(), Some(later + CURSOR_BLINK_PERIOD));
    }
}
