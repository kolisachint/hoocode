//! The TUI status glyphs: one set for every surface.
//!
//! Every running/done/todo mark in the transcript, the comms frame, the task
//! panel and the dialogs comes from [`status`]. The animation frames in
//! [`animation`] cycle the same set, so a spinner never introduces a glyph
//! the font cannot draw.
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
    /// Waiting for the user's approval: `◆` (amber). It ships in the font
    /// (JetBrains Mono covers U+25C6), so the baseline holds.
    pub const WAITING: char = '◆';
    /// Waiting, as a string slice for span and row building.
    pub const WAITING_STR: &str = "◆";
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
        assert_eq!(status::WAITING_STR, "◆");
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
