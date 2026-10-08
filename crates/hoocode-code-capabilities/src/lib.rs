//! Capability index for `SearchHooCode` (`core/capabilities/` in hoocode-ts).
//!
//! The index holds what the session can do: loaded skills, subagent definitions
//! and (later) installed plugins. Lookup is in-process BM25 over code-aware
//! tokens, and an exact name match always ranks first. Docs are not indexed
//! here; see docs/design/semantic-search.md.

mod index;
mod producers;

pub use index::{tokenize, CapabilityIndex};
pub use producers::{load_index, plugin_entries, skill_entries, subagent_entries};

/// What a capability is. `as_str` gives the name the model sees in results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityKind {
    Skill,
    Subagent,
    Plugin,
}

impl CapabilityKind {
    /// The kind as printed in results, e.g. `subagent`.
    pub fn as_str(self) -> &'static str {
        match self {
            CapabilityKind::Skill => "skill",
            CapabilityKind::Subagent => "subagent",
            CapabilityKind::Plugin => "plugin",
        }
    }
}

/// One searchable capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityEntry {
    pub kind: CapabilityKind,
    pub name: String,
    pub description: String,
    /// Where it was loaded from: a file path, or `builtin`.
    pub source: String,
}
