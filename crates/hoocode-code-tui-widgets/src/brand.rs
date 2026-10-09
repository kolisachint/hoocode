//! Brand identity (`core/brand.ts`): the product mark and the glyph vocabulary
//! shared by the footer and the startup summary. Colour is the theme's
//! `accent`; this is the non-colour identity.

/// The mark — a filled hexagon, rendered in the accent colour.
pub const BRAND_MARK: &str = "⬢";

/// Capability classes a session can load, and their glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Skills,
    Commands,
    Agents,
    Mcp,
    Themes,
    Context,
}

impl Category {
    /// `CATEGORY_GLYPH[key]`.
    pub fn glyph(self) -> &'static str {
        match self {
            Category::Skills => "✦",
            Category::Commands => "⌘",
            Category::Agents => "◈",
            Category::Mcp => "⧉",
            Category::Themes => "◒",
            Category::Context => "❯",
        }
    }
}

/// A soft dot separator used between footer/summary segments.
pub const SEGMENT_SEP: &str = "·";

/// Fork glyph preceding a git branch.
pub const GIT_BRANCH_GLYPH: &str = "⑂";
