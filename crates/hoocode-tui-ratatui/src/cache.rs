//! One rendered row, cached: settled rows draw the same lines every frame,
//! so a component keeps its last render keyed by width and theme and only
//! redraws on change, resize or theme switch. Live rows (clocks, streaming
//! text) never enter the cache: their inputs change every tick.

use crate::palette::theme_key;
use crate::palette::ThemeKey;

/// A settled row's last render: its width, theme key and lines.
#[derive(Debug, Default)]
pub struct RowCache {
    width: u16,
    theme: ThemeKey,
    lines: Vec<String>,
    filled: bool,
}

impl RowCache {
    /// The cached lines when they were rendered for `width` under `theme`.
    pub fn get(&self, width: u16, theme: &ThemeKey) -> Option<&[String]> {
        if self.filled && self.width == width && &self.theme == theme {
            Some(&self.lines)
        } else {
            None
        }
    }

    /// Stores a render of `width` under `theme`.
    pub fn set(&mut self, width: u16, theme: ThemeKey, lines: Vec<String>) {
        self.width = width;
        self.theme = theme;
        self.lines = lines;
        self.filled = true;
    }

    /// Drops the render, forcing a redraw.
    pub fn clear(&mut self) {
        self.filled = false;
    }
}

/// The current theme key: what [`RowCache`] keys theme on.
pub fn current_theme_key() -> ThemeKey {
    theme_key()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ThemeKey {
        current_theme_key()
    }

    #[test]
    fn cache_hits_on_same_width_and_theme() {
        let mut cache = RowCache::default();
        assert!(cache.get(80, &key()).is_none());
        cache.set(80, key(), vec!["row".to_string()]);
        assert_eq!(cache.get(80, &key()), Some(["row".to_string()].as_slice()));
    }

    #[test]
    fn cache_invalidates_on_width_change() {
        let mut cache = RowCache::default();
        cache.set(80, key(), vec!["row".to_string()]);
        assert!(cache.get(40, &key()).is_none());
        // A resize stores anew.
        cache.set(40, key(), vec!["narrow".to_string()]);
        assert_eq!(
            cache.get(40, &key()),
            Some(["narrow".to_string()].as_slice())
        );
    }

    #[test]
    fn cache_invalidates_on_theme_change() {
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        let mut cache = RowCache::default();
        cache.set(80, key(), vec!["row".to_string()]);
        // Reloading the theme bumps the generation, so the key differs and
        // the settled row redraws.
        hoocode_code_tui_theme::init_theme(Some("dark"), false);
        assert!(cache.get(80, &key()).is_none());
    }

    #[test]
    fn clear_forces_a_redraw() {
        let mut cache = RowCache::default();
        cache.set(80, key(), vec!["row".to_string()]);
        cache.clear();
        assert!(cache.get(80, &key()).is_none());
    }
}
