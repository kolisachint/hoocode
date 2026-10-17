//! Brand identity (`core/brand.ts`): the glyph vocabulary shared by the footer
//! and the startup summary. Colour is the theme's `accent`; this is the
//! non-colour identity.

/// Capability classes a session can load, and their glyphs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Skills,
    Commands,
    Agents,
    Mcp,
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
            Category::Context => "❯",
        }
    }
}

/// A soft dot separator used between footer/summary segments (one copy, in agent-session).
pub use hoocode_code_agent_session::format::SEGMENT_SEP;

/// Fork glyph preceding a git branch.
pub const GIT_BRANCH_GLYPH: &str = "⑂";
