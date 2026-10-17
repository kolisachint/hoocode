//! Mission Control palette: the chrome tokens (spec §2.3) as ratatui [`Style`]s.
//!
//! The palette reads the current [`hoocode_code_tui_theme`] theme, so
//! `/theme` keeps working, and it follows chalk's colour switch: with colour
//! off (NO_COLOR) the token functions return the default style with no
//! colour, and only the [`bold`]/[`italic`]/[`reversed`] modifiers apply.
//!
//! A theme that does not define a Mission Control token yet (older custom
//! themes) falls back to its legacy token, so every token always resolves.

use std::collections::BTreeMap;

use ratatui_core::style::{Color, Modifier, Style};

/// The Mission Control chrome tokens, each with the legacy theme token it
/// falls back to when a theme does not define it yet.
pub const PALETTE_TOKENS: [(&str, &str); 10] = [
    ("ink", "text"),
    ("dim", "dim"),
    ("faint", "muted"),
    ("acc", "accent"),
    ("ok", "success"),
    ("err", "error"),
    ("warn", "warning"),
    ("you", "agent2"),
    ("vio", "agent2"),
    ("gold", "warning"),
];

/// The legacy token `name` falls back to, if it is a Mission Control token.
fn fallback_for(name: &str) -> Option<&'static str> {
    PALETTE_TOKENS
        .iter()
        .find(|(token, _)| *token == name)
        .map(|(_, legacy)| *legacy)
}

/// A snapshot of the current theme's resolved colours: one JSON parse per
/// snapshot, so a frame holds one [`Palette`] and styles each token from it.
/// The free functions ([`ink`], [`from_token`], …) take a fresh snapshot per
/// call; prefer [`Palette::current`] when styling several tokens.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    colors: BTreeMap<String, String>,
}

/// What a render cache keys on: the theme name plus the generation, so a
/// reload under the same name still invalidates.
pub type ThemeKey = (Option<String>, u64);

/// The current theme key.
pub fn theme_key() -> ThemeKey {
    (
        hoocode_code_tui_theme::current_theme_name(),
        hoocode_code_tui_theme::theme_generation(),
    )
}

/// One JSON parse per theme, not per row: the snapshot for the current
/// theme key is shared until the theme changes.
static SNAPSHOT: std::sync::Mutex<(Option<ThemeKey>, Option<Palette>)> =
    std::sync::Mutex::new((None, None));

impl Palette {
    /// The current process theme's resolved colours (CSS hex per token). An
    /// unknown or in-memory theme resolves to no colours here; [`Palette::style`]
    /// still reads those through the live theme's ANSI. Snapshots are
    /// shared per theme key: hold one [`Palette`] per render pass and pass
    /// it down instead of calling this per row.
    pub fn current() -> Self {
        let key = theme_key();
        if let Ok(guard) = SNAPSHOT.lock() {
            if guard.0.as_ref() == Some(&key) {
                if let Some(palette) = guard.1.clone() {
                    return palette;
                }
            }
        }
        let mut colors = BTreeMap::new();
        if let Some(name) = &key.0 {
            if let Ok(resolved) = hoocode_code_tui_theme::get_resolved_theme_colors(Some(name)) {
                colors = resolved;
            }
        }
        let palette = Self { colors };
        if let Ok(mut guard) = SNAPSHOT.lock() {
            *guard = (Some(key), Some(palette.clone()));
        }
        palette
    }

    /// The ratatui style for `name`: the token's colour, or its legacy
    /// fallback's, or the default style when neither resolves. With colour
    /// off the style is always default (modifiers stay the caller's job).
    pub fn style(&self, name: &str) -> Style {
        if !hoocode_code_tui_theme::chalk::enabled() {
            return Style::default();
        }
        self.color_for(name)
            .map_or_else(Style::default, |color| Style::default().fg(color))
    }

    /// Default text.
    pub fn ink(&self) -> Style {
        self.style("ink")
    }

    /// Dimmed text.
    pub fn dim(&self) -> Style {
        self.style("dim")
    }

    /// Frame lines and faintest text.
    pub fn faint(&self) -> Style {
        self.style("faint")
    }

    /// Accent for live state.
    pub fn acc(&self) -> Style {
        self.style("acc")
    }

    /// Landed and success.
    pub fn ok(&self) -> Style {
        self.style("ok")
    }

    /// Hits and lost comms.
    pub fn err(&self) -> Style {
        self.style("err")
    }

    /// Clearance and amber cards.
    pub fn warn(&self) -> Style {
        self.style("warn")
    }

    /// The user's own missions.
    pub fn you(&self) -> Style {
        self.style("you")
    }

    /// Agent rails and rewind.
    pub fn vio(&self) -> Style {
        self.style("vio")
    }

    /// Step numbers.
    pub fn gold(&self) -> Style {
        self.style("gold")
    }

    /// A background-token style (the `activeToolBg` latest-row stroke):
    /// the token's background colour, or the default style when the theme
    /// sets none. With colour off the style is always default.
    pub fn bg(&self, name: &str) -> Style {
        self.bg_color(name)
            .map_or_else(Style::default, |color| Style::default().bg(color))
    }

    /// A background token's colour, if the theme sets one (and colour is
    /// on). Row builders merge it into their spans' styles.
    pub fn bg_color(&self, name: &str) -> Option<Color> {
        if !hoocode_code_tui_theme::chalk::enabled() {
            return None;
        }
        let theme = hoocode_code_tui_theme::try_theme()?;
        if !theme.has_bg(name) {
            return None;
        }
        bg_color(theme.get_bg_ansi(name))
    }
    /// The hull tint for a paint token (M8e5): the token's colour
    /// darkened for a row background. `None` with colour off or when the
    /// token does not resolve.
    pub fn hull_tint(&self, name: &str) -> Option<Color> {
        if !hoocode_code_tui_theme::chalk::enabled() {
            return None;
        }
        match self.color_for(name) {
            Some(Color::Rgb(r, g, b)) => {
                let dim = |c: u8| ((f32::from(c) * 0.22).round() as u8).max(1);
                Some(Color::Rgb(dim(r), dim(g), dim(b)))
            }
            color => color,
        }
    }
    /// The foreground colour for `name`, following the fallback chain. A
    /// token the theme leaves at its default (`""`) resolves to the
    /// default-text colour: that substitution is load-bearing for chrome
    /// the palette styles directly (the comms glyph keeps both its colour
    /// and its trailing space through it). Call sites porting chalk-styled
    /// rows use an explicit default style instead, matching chalk's
    /// default opener cell for cell.
    fn color_for(&self, name: &str) -> Option<Color> {
        let mut chain = vec![name];
        if let Some(legacy) = fallback_for(name) {
            if legacy != name {
                chain.push(legacy);
            }
        }
        for token in &chain {
            if let Some(css) = self.colors.get(*token) {
                if let Some(color) = css_to_color(css) {
                    return Some(color);
                }
            }
        }
        // In-memory themes never reach the resolved map: read the live theme.
        let theme = hoocode_code_tui_theme::try_theme()?;
        for token in &chain {
            if theme.has(token) {
                let ansi = theme.get_fg_ansi(token);
                if let Some(css) = ansi_to_hex(ansi) {
                    if let Some(color) = css_to_color(&css) {
                        return Some(color);
                    }
                }
            }
        }
        None
    }
}

/// The ratatui style for an open-ended theme token (Mission Control or
/// legacy). Unknown tokens resolve to the default style, never a panic.
pub fn from_token(name: &str) -> Style {
    Palette::current().style(name)
}

/// Default text.
pub fn ink() -> Style {
    from_token("ink")
}

/// Dimmed text.
pub fn dim() -> Style {
    from_token("dim")
}

/// Frame lines and faintest text.
pub fn faint() -> Style {
    from_token("faint")
}

/// Accent for live state.
pub fn acc() -> Style {
    from_token("acc")
}

/// Landed and success.
pub fn ok() -> Style {
    from_token("ok")
}

/// Hits and lost comms.
pub fn err() -> Style {
    from_token("err")
}

/// Clearance and amber cards.
pub fn warn() -> Style {
    from_token("warn")
}

/// The user's own missions.
pub fn you() -> Style {
    from_token("you")
}

/// Agent rails and rewind.
pub fn vio() -> Style {
    from_token("vio")
}

/// Step numbers.
pub fn gold() -> Style {
    from_token("gold")
}

/// Adds bold to a palette style (kept under NO_COLOR: modifiers still apply).
pub fn bold(style: Style) -> Style {
    style.add_modifier(Modifier::BOLD)
}

/// Adds italic to a palette style.
pub fn italic(style: Style) -> Style {
    style.add_modifier(Modifier::ITALIC)
}

/// Adds reverse video to a palette style.
pub fn reversed(style: Style) -> Style {
    style.add_modifier(Modifier::REVERSED)
}

/// A CSS hex colour as a ratatui colour (`None` when it does not parse).
fn css_to_color(css: &str) -> Option<Color> {
    let (r, g, b) = hoocode_code_tui_theme::hex_to_rgb(css).ok()?;
    Some(Color::Rgb(
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
    ))
}

/// A background SGR opener as a colour: `\x1b[48;2;r;g;bm` or
/// `\x1b[48;5;nm`. Anything else (including the default `\x1b[49m`) is
/// `None`: no colour.
fn bg_color(ansi: &str) -> Option<Color> {
    let body = ansi.strip_prefix("\x1b[")?.strip_suffix('m')?;
    let mut parts = body.split(';');
    match (parts.next()?, parts.next()?) {
        ("48", "2") => Some(Color::Rgb(
            parts.next()?.parse::<u8>().ok()?,
            parts.next()?.parse::<u8>().ok()?,
            parts.next()?.parse::<u8>().ok()?,
        )),
        ("48", "5") => parts.next()?.parse::<u8>().ok().map(Color::Indexed),
        _ => None,
    }
}

/// A foreground SGR opener as CSS hex: `\x1b[38;2;r;g;bm` or
/// `\x1b[38;5;nm`. The default opener (`\x1b[39m`) is `None`: no colour.
fn ansi_to_hex(ansi: &str) -> Option<String> {
    let body = ansi.strip_prefix("\x1b[")?.strip_suffix('m')?;
    let mut parts = body.split(';');
    match (parts.next()?, parts.next()?) {
        ("38", "2") => {
            let r = parts.next()?.parse::<u8>().ok()?;
            let g = parts.next()?.parse::<u8>().ok()?;
            let b = parts.next()?.parse::<u8>().ok()?;
            Some(format!("#{r:02x}{g:02x}{b:02x}"))
        }
        ("38", "5") => parts
            .next()?
            .parse::<i64>()
            .ok()
            .map(hoocode_code_tui_theme::ansi256_to_hex),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_hex_parses_to_rgb() {
        assert_eq!(css_to_color("#9b8cff"), Some(Color::Rgb(0x9b, 0x8c, 0xff)));
        assert_eq!(css_to_color("nope"), None);
        assert_eq!(css_to_color(""), None);
    }

    #[test]
    fn sgr_openers_parse_to_hex() {
        assert_eq!(
            ansi_to_hex("\x1b[38;2;155;140;255m"),
            Some("#9b8cff".to_string())
        );
        assert_eq!(
            ansi_to_hex("\x1b[38;5;99m"),
            Some(hoocode_code_tui_theme::ansi256_to_hex(99))
        );
        assert_eq!(ansi_to_hex("\x1b[39m"), None);
        assert_eq!(ansi_to_hex("plain"), None);
    }

    #[test]
    fn every_token_has_a_legacy_fallback() {
        for (token, legacy) in PALETTE_TOKENS {
            assert!(!token.is_empty() && !legacy.is_empty());
            assert_eq!(fallback_for(token), Some(legacy));
        }
        assert_eq!(fallback_for("accent"), None);
    }
}
