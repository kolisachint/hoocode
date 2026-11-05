//! Key sequences. A binding such as `"ctrl+x m"` has two steps, separated by
//! whitespace. Each step is one key identifier, parsed as a [`KeySpec`].
//!
//! A single uppercase letter step (`"M"`) means Shift+letter, so it matches
//! the typed capital and not the lowercase key.

use std::fmt;

use crate::matching::KeySpec;

/// Why a key sequence string does not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceError {
    /// The text has no steps (empty or only whitespace).
    Empty,
    /// A step is not a key identifier (for example `"ctrl+"`).
    BadStep(String),
    /// The sequence does not start with the leader key.
    NoLeader(String),
    /// The sequence is only the leader key, with no step after it.
    LeaderOnly(String),
    /// A step after the first is the leader or Esc, which cancel a pending
    /// sequence, so no binding can use them there.
    ReservedStep(String),
}

impl fmt::Display for SequenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "empty key sequence"),
            Self::BadStep(step) => write!(f, "not a key: {step:?}"),
            Self::NoLeader(seq) => write!(f, "sequence {seq:?} must start with the leader key"),
            Self::LeaderOnly(seq) => write!(f, "sequence {seq:?} needs a key after the leader"),
            Self::ReservedStep(seq) => write!(
                f,
                "sequence {seq:?} uses the leader or esc after the leader, which cancels"
            ),
        }
    }
}

impl std::error::Error for SequenceError {}

/// One step of a key sequence: the key id as written, and its matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStep {
    id: String,
    spec: KeySpec,
}

impl KeyStep {
    /// Parses one step. A single uppercase letter becomes Shift+letter.
    pub fn parse(text: &str) -> Result<Self, SequenceError> {
        let mut chars = text.chars();
        let spec_text = match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_uppercase() => {
                format!("shift+{}", c.to_ascii_lowercase())
            }
            _ => text.to_string(),
        };
        let spec =
            KeySpec::parse(&spec_text).ok_or_else(|| SequenceError::BadStep(text.to_string()))?;
        Ok(Self {
            id: text.to_string(),
            spec,
        })
    }

    /// The key id as it was written, for example `"ctrl+x"` or `"M"`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Whether raw terminal input `data` is a press of this step's key.
    pub fn matches(&self, data: &str) -> bool {
        self.spec.matches(data)
    }

    /// The matcher, for comparisons between steps.
    pub(crate) fn spec(&self) -> &KeySpec {
        &self.spec
    }

    /// Whether this step types a character (see [`KeySpec::is_printable`]).
    pub fn is_printable(&self) -> bool {
        self.spec.is_printable()
    }
}

/// A parsed key sequence: one or more steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySequence {
    steps: Vec<KeyStep>,
}

impl KeySequence {
    /// Parses `"ctrl+x m"` into two steps. Steps are separated by whitespace.
    pub fn parse(text: &str) -> Result<Self, SequenceError> {
        let steps = text
            .split_whitespace()
            .map(KeyStep::parse)
            .collect::<Result<Vec<_>, _>>()?;
        if steps.is_empty() {
            return Err(SequenceError::Empty);
        }
        Ok(Self { steps })
    }

    /// The steps, in order. Never empty.
    pub fn steps(&self) -> &[KeyStep] {
        &self.steps
    }
}
