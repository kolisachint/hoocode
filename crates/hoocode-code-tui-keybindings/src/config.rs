//! `keybindings.json`, both forms, and what it means.
//!
//! Flat form (the original): `{"app.model.cycleForward": "alt+m"}`, where a
//! value is one key or a list of keys. A multi-step value such as
//! `"ctrl+x m"` is a leader sequence, bound in the Prompt context.
//!
//! Grouped form: a top-level `"leader"` key (default `ctrl+x`),
//! `"leaderTimeoutMs"` (default 1500), and `"bindings"`, a list of
//! `{"context": ..., "bindings": {key: action}}`. A `null` or `"none"`
//! action unbinds the key in that context.
//!
//! Parsing never fails. A bad part is skipped and reported in `warnings`,
//! and the rest of the file still applies.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

use hoocode_tui_keys::{KeySequence, LeaderKey, DEFAULT_LEADER_TIMEOUT};
use serde_json::{Map, Value};

/// The leader key when the file does not name one.
pub const DEFAULT_LEADER: &str = "ctrl+x";

/// Keys no binding may take: the terminal's own control keys that hoocode
/// relies on. A binding to one, or a leader that is one, is skipped with an error.
pub const RESERVED_KEYS: [&str; 7] = [
    "ctrl+c", "ctrl+d", "ctrl+z", "ctrl+m", "ctrl+i", "ctrl+[", "ctrl+h",
];

/// Keys the terminal or the shell often takes (flow control, the NUL of
/// Ctrl+Space). They can be bound, with a caution that they may never arrive.
pub const WARNED_KEYS: [&str; 4] = ["ctrl+s", "ctrl+q", "ctrl+b", "ctrl+space"];

/// Top-level keys starting with `$` are comments for a reader (the defaults
/// file's `$comment` and `$reserved`). The loader ignores them.
const COMMENT_PREFIX: char = '$';

/// A scope in which bindings apply. A binding in `Global` applies everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Context {
    Global,
    Prompt,
    /// Keys typed after the leader. Write them without the leader (`"m"`).
    Leader,
    Scroll,
    Picker,
    Panel,
    Dialog,
    Tree,
    Models,
}

impl Context {
    /// Every context, in the order the docs list them.
    pub const ALL: [Context; 9] = [
        Context::Global,
        Context::Prompt,
        Context::Leader,
        Context::Scroll,
        Context::Picker,
        Context::Panel,
        Context::Dialog,
        Context::Tree,
        Context::Models,
    ];

    /// The name used in `keybindings.json`.
    pub fn name(self) -> &'static str {
        match self {
            Context::Global => "Global",
            Context::Prompt => "Prompt",
            Context::Leader => "Leader",
            Context::Scroll => "Scroll",
            Context::Picker => "Picker",
            Context::Panel => "Panel",
            Context::Dialog => "Dialog",
            Context::Tree => "Tree",
            Context::Models => "Models",
        }
    }

    /// Reads a context name, exactly as the docs spell it.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// One binding from the grouped form. `action: None` unbinds the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextOverride {
    pub context: Context,
    /// The key or sequence, with single spaces (`"ctrl+x m"`).
    pub key: String,
    pub action: Option<String>,
}

/// A parsed `keybindings.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapConfig {
    pub leader: LeaderKey,
    pub leader_timeout: Duration,
    /// Flat bindings, one list of single keys per action id. An empty list unbinds the action.
    pub flat: HashMap<String, Vec<String>>,
    /// Bindings from the grouped form, and sequences from the flat form.
    pub overrides: Vec<ContextOverride>,
    /// Problems found while reading the file. Each one names what was skipped.
    pub warnings: Vec<String>,
    /// Bindings that are kept but may not work: a warned key (see [`WARNED_KEYS`]).
    pub cautions: Vec<String>,
}

/// Collapses runs of whitespace, so `"ctrl+x  m"` and `"ctrl+x m"` are one key.
pub fn normalize_key(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn default_leader() -> LeaderKey {
    // A constant that is a valid, non-printable key.
    LeaderKey::new(DEFAULT_LEADER).unwrap_or_else(|_| unreachable!("ctrl+x is a valid leader"))
}

impl KeymapConfig {
    /// The config with no bindings, the default leader, and no warnings.
    pub fn defaults() -> Self {
        Self {
            leader: default_leader(),
            leader_timeout: DEFAULT_LEADER_TIMEOUT,
            flat: HashMap::new(),
            overrides: Vec::new(),
            warnings: Vec::new(),
            cautions: Vec::new(),
        }
    }

    /// Reads a file. `None` when the file does not exist. A file that
    /// cannot be read, or is not a JSON object, gives the defaults with a warning.
    pub fn load(path: &Path) -> Option<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Some(Self::from_json_str(&text)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => Some(Self::with_warning(format!(
                "cannot read {}: {err}; using the defaults",
                path.display()
            ))),
        }
    }

    /// Parses the text of a file. Bad JSON or a non-object gives the defaults with a warning.
    pub fn from_json_str(text: &str) -> Self {
        match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(map)) => Self::from_object(&map),
            Ok(_) => Self::with_warning(
                "keybindings.json must be a JSON object; using the defaults".into(),
            ),
            Err(err) => Self::with_warning(format!(
                "keybindings.json is not valid JSON ({err}); using the defaults"
            )),
        }
    }

    fn with_warning(warning: String) -> Self {
        let mut config = Self::defaults();
        config.warnings.push(warning);
        config
    }

    fn from_object(map: &Map<String, Value>) -> Self {
        let mut config = Self::defaults();
        config.read_leader(map);
        config.read_timeout(map);
        config.read_flat(map);
        config.read_groups(map);
        config
    }

    fn read_leader(&mut self, map: &Map<String, Value>) {
        let Some(value) = map.get("leader") else {
            return;
        };
        let Some(text) = value.as_str() else {
            self.warnings
                .push("\"leader\" must be a key such as \"ctrl+x\"; using ctrl+x".into());
            return;
        };
        match LeaderKey::new(text) {
            Ok(leader) if self.allow_key("leader", text) => self.leader = leader,
            Ok(_) => {}
            Err(err) => self
                .warnings
                .push(format!("leader rejected ({err}); using ctrl+x")),
        }
    }

    fn read_timeout(&mut self, map: &Map<String, Value>) {
        let Some(value) = map.get("leaderTimeoutMs") else {
            return;
        };
        match value.as_u64().filter(|ms| *ms >= 1) {
            Some(ms) => self.leader_timeout = Duration::from_millis(ms),
            None => self.warnings.push(
                "\"leaderTimeoutMs\" must be a whole number of milliseconds of 1 or more; using 1500".into(),
            ),
        }
    }

    /// The flat form: every top-level key except the reserved ones and comments.
    fn read_flat(&mut self, map: &Map<String, Value>) {
        for (action, value) in map {
            if matches!(action.as_str(), "leader" | "leaderTimeoutMs" | "bindings")
                || action.starts_with(COMMENT_PREFIX)
            {
                continue;
            }
            let keys = match value {
                Value::Null => Some(Vec::new()),
                Value::String(s) if s == "none" => Some(Vec::new()),
                Value::String(s) => Some(vec![s.clone()]),
                Value::Array(items) if items.iter().all(Value::is_string) => Some(
                    items
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                ),
                _ => None,
            };
            let Some(keys) = keys else {
                self.warnings.push(format!(
                    "{action}: a binding must be a key, a list of keys, or null; skipped"
                ));
                continue;
            };
            let mut single = Vec::new();
            for key in keys {
                if KeySequence::parse(&key).is_err() {
                    self.warnings
                        .push(format!("{action}: {key:?} is not a key; skipped"));
                } else if self.is_sequence(&key) {
                    self.add_sequence(Context::Prompt, &key, Some(action.clone()));
                } else if self.allow_key(action, &key) {
                    single.push(key);
                }
            }
            self.flat.insert(action.clone(), single);
        }
        self.check_flat_conflicts();
    }

    /// A key listed for two actions in the flat form. Both are kept, and the
    /// first id in sorted order runs when the key is pressed. Reported once per key.
    fn check_flat_conflicts(&mut self) {
        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (action, keys) in &self.flat {
            for key in keys {
                owners
                    .entry(normalize_key(key).to_lowercase())
                    .or_default()
                    .push(action.clone());
            }
        }
        for (key, mut actions) in owners {
            if actions.len() > 1 {
                actions.sort();
                self.warnings.push(format!(
                    "{key}: listed for {} in the file; the first id in sorted order runs",
                    actions.join(" and ")
                ));
            }
        }
    }

    /// Whether a single key may be bound, for the file's `label` (a context or
    /// an action). A reserved key is skipped with an error. A warned key is kept
    /// with a caution.
    fn allow_key(&mut self, label: &str, key: &str) -> bool {
        let lower = normalize_key(key).to_lowercase();
        if RESERVED_KEYS.contains(&lower.as_str()) {
            self.warnings.push(format!(
                "{label}: {key} is reserved and cannot be rebound; skipped"
            ));
            return false;
        }
        if WARNED_KEYS.contains(&lower.as_str()) {
            self.cautions.push(format!(
                "{label}: {key} is often taken by the terminal or the shell, so it may never reach hoocode"
            ));
        }
        true
    }

    /// Adds a binding in a context. A second binding of the same key in the same
    /// context replaces the first, with an error when the actions differ.
    fn push_override(&mut self, new: ContextOverride) {
        if let Some(pos) = self
            .overrides
            .iter()
            .position(|o| o.context == new.context && o.key == new.key)
        {
            let earlier = self.overrides.remove(pos);
            if earlier.action != new.action {
                self.warnings.push(format!(
                    "{}: {} is bound twice in the file ({} and {}); the first is skipped",
                    new.context.name(),
                    new.key,
                    action_text(&earlier.action),
                    action_text(&new.action),
                ));
            }
        }
        self.overrides.push(new);
    }

    /// The grouped form: `"bindings": [{"context": ..., "bindings": {...}}]`.
    fn read_groups(&mut self, map: &Map<String, Value>) {
        let Some(groups) = map.get("bindings") else {
            return;
        };
        let Some(groups) = groups.as_array() else {
            self.warnings.push(
                "\"bindings\" must be a list of {\"context\", \"bindings\"} groups; skipped".into(),
            );
            return;
        };
        for group in groups {
            self.read_group(group);
        }
    }

    fn read_group(&mut self, group: &Value) {
        let Some(group) = group.as_object() else {
            self.warnings
                .push("a binding group must be an object; skipped".into());
            return;
        };
        let name = group
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(context) = Context::parse(name) else {
            self.warnings.push(format!(
                "unknown context {name:?}; its bindings are skipped"
            ));
            return;
        };
        let Some(bindings) = group.get("bindings").and_then(Value::as_object) else {
            self.warnings.push(format!(
                "context {name}: \"bindings\" must be an object; skipped"
            ));
            return;
        };
        for (key, value) in bindings {
            let action = match value {
                Value::Null => None,
                Value::String(s) if s == "none" => None,
                Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => {
                    self.warnings.push(format!(
                        "{name} {key:?}: the action must be an action id, or null or \"none\"; skipped"
                    ));
                    continue;
                }
            };
            self.add_grouped(context, key, action);
        }
    }

    fn add_grouped(&mut self, context: Context, key: &str, action: Option<String>) {
        let key = normalize_key(key);
        let Ok(sequence) = KeySequence::parse(&key) else {
            self.warnings
                .push(format!("{}: {key:?} is not a key; skipped", context.name()));
            return;
        };
        let is_sequence = sequence.steps().len() > 1;
        if context == Context::Leader {
            // Written after the leader: one key, and not the leader itself.
            if is_sequence || self.is_leader_step(&key) {
                self.warnings.push(format!(
                    "Leader {key:?}: write the key after the leader, e.g. \"m\"; skipped"
                ));
                return;
            }
            self.push_override(ContextOverride {
                context,
                key,
                action,
            });
        } else if is_sequence {
            self.add_sequence(context, &key, action);
        } else if self.is_leader_step(&key) {
            self.warnings.push(format!(
                "{}: {key:?} is the leader key; bind a sequence after it, e.g. \"{key} m\"; skipped",
                context.name()
            ));
        } else if self.allow_key(context.name(), &key) {
            self.push_override(ContextOverride {
                context,
                key,
                action,
            });
        }
    }

    /// Whether `key` (a whole value) is a multi-step sequence.
    fn is_sequence(&self, key: &str) -> bool {
        KeySequence::parse(key).is_ok_and(|s| s.steps().len() > 1)
    }

    /// Whether `key` is only the leader key.
    fn is_leader_step(&self, key: &str) -> bool {
        KeySequence::parse(key)
            .is_ok_and(|s| s.steps().len() == 1 && self.is_leader_id(s.steps()[0].id()))
    }

    /// Whether a key id names the leader key (case does not matter).
    fn is_leader_id(&self, id: &str) -> bool {
        id.eq_ignore_ascii_case(self.leader.id())
    }

    /// Adds a multi-step sequence, if it starts with the leader.
    fn add_sequence(&mut self, context: Context, key: &str, action: Option<String>) {
        let key = normalize_key(key);
        let starts_with_leader =
            KeySequence::parse(&key).is_ok_and(|s| self.is_leader_id(s.steps()[0].id()));
        if !starts_with_leader {
            self.warnings.push(format!(
                "{key:?} is a sequence, so it must start with the leader ({}); skipped",
                self.leader.id()
            ));
            return;
        }
        self.push_override(ContextOverride {
            context,
            key,
            action,
        });
    }
}

/// The action a binding names, for a message: `nothing (unbound)` for `null`.
fn action_text(action: &Option<String>) -> &str {
    action.as_deref().unwrap_or("nothing (unbound)")
}

/// The effective per-key table for one context: `Global` first, then the
/// context's own entries, which win. `None` means the key is unbound there.
pub fn effective_overrides(
    overrides: &[ContextOverride],
    context: Context,
) -> BTreeMap<String, Option<String>> {
    let mut table = BTreeMap::new();
    for o in overrides.iter().filter(|o| o.context == Context::Global) {
        table.insert(o.key.clone(), o.action.clone());
    }
    for o in overrides
        .iter()
        .filter(|o| o.context == context && context != Context::Global)
    {
        table.insert(o.key.clone(), o.action.clone());
    }
    table
}
