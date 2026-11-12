//! The TUI status glyphs: one set for every surface.
//!
//! Every running/done/todo mark in the transcript, the prompt row, the task
//! panel and the dialogs comes from [`status`]. The animation frames in
//! [`animation`] cycle the same set, so a spinner never introduces a glyph
//! the font cannot draw.
//!
//! The one pair outside the circles is [`status::CLOSED`] and
//! [`status::OPEN`], the state of a block that opens in place.
//!
//! Glyph choice is a font-coverage decision, not taste: the half-filled
//! circles (`◐◑◒◓`) are missing from JetBrainsMono Nerd Font Mono, so the
//! terminal falls back to another font and the mark sits off-baseline next
//! to `●`/`○`. The fisheye `◉` ships in the font and keeps the baseline.

/// The status circles: done, running, todo. Shapes differ, so they read
/// under `NO_COLOR`; the palette adds green/amber/faint on top.
pub mod status {
    /// Done: `●` (green).
    pub const DONE: char = '●';
    /// Done, as a string slice for span and row building.
    pub const DONE_STR: &str = "●";
    /// Running: `◉` (amber). `◐` is deliberately not used: it is missing
    /// from the font and falls back off-baseline.
    pub const RUNNING: char = '◉';
    /// Running, as a string slice for span and row building.
    pub const RUNNING_STR: &str = "◉";
    /// Todo: `○` (faint).
    pub const TODO: char = '○';
    /// Todo, as a string slice for span and row building.
    pub const TODO_STR: &str = "○";
    /// Failed, stopped or denied: `⊗` (red).
    pub const FAILED: char = '⊗';
    /// Failed, as a string slice for span and row building.
    pub const FAILED_STR: &str = "⊗";
    /// The marker before a user line, a selected row and the prompt: `›`.
    pub const MARKER: char = '›';
    /// The marker, as a string slice for span and row building.
    pub const MARKER_STR: &str = "›";

    /// A block that can open and is closed: `▸` (dim), after the header's
    /// last fact. With [`OPEN`] the one pair outside the status circles
    /// (added 2026-11-11): the row's state had no shape before, and no glyph
    /// of the set reads as "closed" against "open". Both are one cell wide
    /// and ship in JetBrains Mono (checked with `fc-query`).
    pub const CLOSED: char = '▸';
    /// Closed, as a string slice for span and row building.
    pub const CLOSED_STR: &str = "▸";
    /// A block that is open: `▾` (dim), in the same place.
    pub const OPEN: char = '▾';
    /// Open, as a string slice for span and row building.
    pub const OPEN_STR: &str = "▾";

    /// ASCII stand-ins for terminals without the glyphs above (for
    /// `NO_COLOR`-style plain output or a font without coverage).
    pub mod ascii {
        /// Done.
        pub const DONE: &str = "*";
        /// Failed, stopped or denied.
        pub const FAILED: &str = "x";
        /// Working.
        pub const RUNNING: &str = "@";
        /// Todo or queued.
        pub const TODO: &str = "o";
        /// Marker.
        pub const MARKER: &str = ">";
        /// A closed block that can open.
        pub const CLOSED: &str = "+";
        /// An open block.
        pub const OPEN: &str = "-";
    }
}

/// The running-mark animation: a pulse through the status set, driven by the
/// existing one-second elapsed-time tick. No new timer, no extra frames.
/// Call sites keep the still [`status::RUNNING`] mark by default; this is the
/// opt-in motion version.
pub mod animation {
    use super::status;

    /// One pulse loop: full, fisheye, hollow, fisheye. All three shapes ship
    /// in the font, so every frame holds the baseline.
    pub const RUNNING_FRAMES: [char; 4] =
        [status::DONE, status::RUNNING, status::TODO, status::RUNNING];

    /// The frame for this second of a run. Wraps, so any elapsed count works.
    pub fn running_frame(elapsed_secs: u64) -> char {
        RUNNING_FRAMES[elapsed_secs as usize % RUNNING_FRAMES.len()]
    }

    /// The running mark: the still [`status::RUNNING`] glyph, or the pulse
    /// frame when `animated`.
    pub fn running_glyph(animated: bool, elapsed_secs: u64) -> char {
        if animated {
            running_frame(elapsed_secs)
        } else {
            status::RUNNING
        }
    }
}

#[cfg(test)]
mod tests {
    use super::animation::{running_frame, running_glyph, RUNNING_FRAMES};
    use super::status;

    #[test]
    fn status_glyphs_are_the_documented_shapes() {
        assert_eq!(status::DONE, '●');
        assert_eq!(status::RUNNING, '◉');
        assert_eq!(status::TODO, '○');
        assert_eq!(status::DONE_STR, "●");
        assert_eq!(status::RUNNING_STR, "◉");
        assert_eq!(status::TODO_STR, "○");
        assert_eq!(status::FAILED, '⊗');
        assert_eq!(status::FAILED_STR, "⊗");
        assert_eq!(status::MARKER, '›');
        assert_eq!(status::MARKER_STR, "›");
        assert!(status::ascii::DONE.is_ascii() && status::ascii::MARKER.is_ascii());
        assert_eq!(status::CLOSED, '▸');
        assert_eq!(status::OPEN, '▾');
        assert_eq!(status::CLOSED_STR, "▸");
        assert_eq!(status::OPEN_STR, "▾");
        assert!(status::ascii::CLOSED.is_ascii() && status::ascii::OPEN.is_ascii());
        assert_ne!(status::ascii::CLOSED, status::ascii::OPEN);
    }

    #[test]
    fn frames_cycle_and_wrap() {
        assert_eq!(RUNNING_FRAMES.len(), 4);
        assert_eq!(running_frame(0), '●');
        assert_eq!(running_frame(1), '◉');
        assert_eq!(running_frame(2), '○');
        assert_eq!(running_frame(3), '◉');
        assert_eq!(running_frame(4), running_frame(0));
        assert_eq!(running_frame(101), running_frame(101 % 4));
    }

    #[test]
    fn still_mode_holds_the_running_mark() {
        for secs in [0, 1, 2, 3, 60] {
            assert_eq!(running_glyph(false, secs), status::RUNNING);
        }
        assert_eq!(running_glyph(true, 2), '○');
    }
}
