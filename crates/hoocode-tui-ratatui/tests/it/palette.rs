//! Golden: the Mission Control palette on the dark theme, plus the
//! fallback and NO_COLOR behavior. Every test that touches the
//! process-wide theme or chalk's colour switch holds the lock, so those
//! tests run one at a time.

use std::sync::{Mutex, MutexGuard};

use hoocode_code_tui_theme::{
    chalk, create_theme, current_theme_name, load_theme_json, set_theme_instance, ColorMode,
};
use hoocode_tui_ratatui::core::buffer::Buffer;
use hoocode_tui_ratatui::core::layout::Rect;
use hoocode_tui_ratatui::core::style::{Modifier, Style};
use hoocode_tui_ratatui::{
    acc, bold, dim, err, faint, from_token, gold, ink, italic, ok, reversed, vio, warn, you,
    Palette, RatatuiComponent, PALETTE_TOKENS,
};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;

/// Serializes the tests that switch the process-wide theme or chalk.
fn palette_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Restores chalk's colour switch on drop, so a failing assert cannot leave
/// colour off for the other tests in this binary.
struct ChalkGuard;

impl Drop for ChalkGuard {
    fn drop(&mut self) {
        chalk::set_enabled(true);
    }
}

/// The bundled dark theme as the current theme, in truecolor so the golden
/// holds whatever the test environment detects.
fn use_dark_truecolor() {
    chalk::set_enabled(true);
    let json = load_theme_json("dark").expect("the bundled dark theme loads");
    let theme =
        create_theme(&json, Some(ColorMode::Truecolor), None).expect("the dark theme builds");
    set_theme_instance(theme);
}

/// The bundled dark theme with the Mission Control tokens stripped, so the
/// legacy fallbacks show.
fn use_dark_without_palette_tokens() {
    chalk::set_enabled(true);
    let mut json = load_theme_json("dark").expect("the bundled dark theme loads");
    json.colors
        .retain(|(name, _)| !PALETTE_TOKENS.iter().any(|(token, _)| *token == name));
    let theme =
        create_theme(&json, Some(ColorMode::Truecolor), None).expect("the dark theme builds");
    set_theme_instance(theme);
}

/// One labelled palette style per sheet row.
type PaletteRow = (&'static str, fn() -> Style);

fn rows() -> [PaletteRow; 11] {
    [
        ("ink", ink),
        ("dim", dim),
        ("faint", faint),
        ("acc", acc),
        ("ok", ok),
        ("err", err),
        ("warn", warn),
        ("you", you),
        ("vio", vio),
        ("gold", gold),
        ("bold", || bold(acc())),
    ]
}

fn palette_sheet(area: Rect, buf: &mut Buffer) {
    for (y, (label, style)) in rows().iter().enumerate() {
        let line = format!("{label} ▲ M1 ◐ working");
        buf.set_string(area.x, area.y + y as u16, &line, style());
    }
}

#[test]
fn palette_dark_sheet() {
    let _lock = palette_lock();
    let _chalk = ChalkGuard;
    use_dark_truecolor();
    assert_eq!(current_theme_name().as_deref(), Some("<in-memory>"));
    let mut component = RatatuiComponent::new(11, palette_sheet);
    assert_golden!("palette_dark", render_golden(&mut component, 30));
}

#[test]
fn legacy_tokens_cover_themes_without_palette_tokens() {
    let _lock = palette_lock();
    let _chalk = ChalkGuard;
    use_dark_without_palette_tokens();
    let palette = Palette::current();
    assert_eq!(palette.style("acc"), palette.style("accent"));
    assert_eq!(palette.style("ok"), palette.style("success"));
    assert_eq!(palette.style("err"), palette.style("error"));
    assert_eq!(palette.style("warn"), palette.style("warning"));
    assert_eq!(palette.style("you"), palette.style("agent2"));
    assert_eq!(palette.style("vio"), palette.style("agent2"));
    assert_eq!(palette.style("gold"), palette.style("warning"));
    assert_eq!(palette.style("ink"), palette.style("text"));
}

#[test]
fn unknown_tokens_are_the_default_style() {
    let _lock = palette_lock();
    let _chalk = ChalkGuard;
    use_dark_truecolor();
    assert_eq!(from_token("nope"), Style::default());
    assert_eq!(Palette::current().style(""), Style::default());
}

#[test]
fn no_color_means_modifiers_only() {
    let _lock = palette_lock();
    let _chalk = ChalkGuard;
    use_dark_truecolor();
    chalk::set_enabled(false);
    for (label, style) in rows() {
        if label == "bold" {
            continue;
        }
        assert_eq!(style(), Style::default(), "{label} has no colour");
    }
    assert_eq!(from_token("acc"), Style::default());
    assert_eq!(bold(ink()), Style::default().add_modifier(Modifier::BOLD));
    assert_eq!(
        italic(dim()),
        Style::default().add_modifier(Modifier::ITALIC)
    );
    assert_eq!(
        reversed(err()),
        Style::default().add_modifier(Modifier::REVERSED)
    );
}
