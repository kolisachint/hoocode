//! The keyboard map (`core/keybindings.ts`).
//!
//! Bindings are grouped by intention — Flow, Compose, Steer, Read,
//! Scroll, Go, then the overlays that print their own keys — and the
//! declaration order *is* the grouping: `keybindings.json` is written in it.
//! The layout rules (alt sets a value, ctrl acts on what is drawn, a dial
//! steps back with shift, no bare `shift+<letter>`) are held by the
//! `keybinding_layout` tests; the rationale for each key is in hoocode's
//! `core/keybindings.ts`.

use std::collections::{BTreeMap, HashMap};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use hoocode_tui_keys::{
    KeybindingDefinition, KeybindingsManager, LeaderKey, LeaderMachine, TUI_KEYBINDINGS,
};
use serde_json::{Map, Value};

use crate::config::{effective_overrides, normalize_key, Context, ContextOverride, KeymapConfig};

/// One binding: id, default keys, description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingEntry {
    pub id: &'static str,
    pub default_keys: Vec<&'static str>,
    pub description: &'static str,
}

fn entry(id: &'static str, keys: &[&'static str], description: &'static str) -> KeybindingEntry {
    KeybindingEntry {
        id,
        default_keys: keys.to_vec(),
        description,
    }
}

/// The app bindings, in declaration order (after the TUI library's).
pub fn app_keybindings() -> Vec<KeybindingEntry> {
    let windows = cfg!(windows);
    vec![
        // ── Flow — getting out, getting back
        entry("app.interrupt", &["escape"], "Cancel or abort"),
        entry("app.clear", &["ctrl+c"], "Clear editor"),
        entry("app.exit", &["ctrl+d"], "Exit when editor is empty"),
        entry(
            "app.suspend",
            if windows { &[] } else { &["ctrl+z"] },
            "Suspend to background",
        ),
        // ── Compose — the message in your hands
        entry("app.editor.external", &["alt+e"], "Open external editor"),
        entry(
            "app.input.voiceTranscribe",
            &["alt+r"],
            "Record voice and transcribe into the editor",
        ),
        entry(
            "app.clipboard.pasteImage",
            if windows { &["alt+v"] } else { &["ctrl+v"] },
            "Paste image from clipboard",
        ),
        entry(
            "app.clipboard.copyMessage",
            &["alt+q"],
            "Copy the agent's last message to the clipboard",
        ),
        entry(
            "app.message.followUp",
            &["alt+enter"],
            "Queue follow-up message",
        ),
        entry(
            "app.message.dequeue",
            &["alt+up"],
            "Restore queued messages",
        ),
        // ── Steer — what the agent is before it runs
        entry(
            "app.mode.cycleForward",
            &["shift+tab", "alt+a"],
            "Cycle agent mode (ask → plan → build → debug)",
        ),
        entry(
            "app.mode.cycleBackward",
            &["shift+alt+a"],
            "Cycle agent mode backward",
        ),
        entry("app.model.cycleForward", &["alt+m"], "Cycle to next model"),
        entry(
            "app.model.cycleBackward",
            &["shift+alt+m"],
            "Cycle to previous model",
        ),
        entry("app.model.select", &[], "Open model selector"),
        entry(
            "app.thinking.cycleForward",
            &["alt+t"],
            "Cycle thinking level (off → … → high)",
        ),
        entry(
            "app.thinking.cycleBackward",
            &["shift+alt+t"],
            "Cycle thinking level backward",
        ),
        // ── Read — what you see of what it did
        entry(
            "app.view.cycleForward",
            &["alt+o"],
            "Cycle tool output view (outline → peek → full)",
        ),
        entry(
            "app.view.cycleBackward",
            &["shift+alt+o"],
            "Cycle tool output view backward",
        ),
        entry(
            "app.tools.expand",
            &["ctrl+o"],
            "Step tool output: outline → peek → full → outline",
        ),
        entry("app.thinking.toggle", &["ctrl+t"], "Toggle thinking blocks"),
        entry(
            "app.tasks.cycleForward",
            &["alt+l"],
            "Cycle task panel tab (Plan → Agents → Shell → Schedule → MCP)",
        ),
        entry(
            "app.tasks.cycleBackward",
            &["shift+alt+l"],
            "Cycle task panel view backward",
        ),
        entry(
            "app.panel.focus",
            &["ctrl+g"],
            "Show or hide the task panel",
        ),
        // ── Scroll — where in the transcript you are looking
        entry(
            "app.scroll.pageUp",
            &["pageUp"],
            "Scroll the transcript up a page (pins the view)",
        ),
        entry(
            "app.scroll.pageDown",
            &["pageDown"],
            "Scroll the transcript down a page",
        ),
        entry(
            "app.scroll.top",
            &["ctrl+home"],
            "Jump to the start of the transcript",
        ),
        entry(
            "app.scroll.bottom",
            &["ctrl+end"],
            "Jump back to live output",
        ),
        entry("app.scroll.search", &["ctrl+r"], "Search the transcript"),
        entry(
            "app.scroll.previousMessage",
            &["ctrl+up"],
            "Jump to your previous message",
        ),
        entry(
            "app.scroll.nextMessage",
            &["ctrl+down"],
            "Jump to your next message",
        ),
        entry(
            "app.scroll.shiftLineUp",
            &["shift+up"],
            "Scroll up a line (pinned view)",
        ),
        entry(
            "app.scroll.shiftLineDown",
            &["shift+down"],
            "Scroll down a line (pinned view)",
        ),
        // ── Go — sessions and places
        entry(
            "app.session.resume",
            &["alt+h"],
            "Resume a session from history",
        ),
        entry("app.session.tree", &[], "Open session tree"),
        entry("app.session.new", &[], "Start a new session"),
        entry("app.session.fork", &[], "Fork current session"),
        entry(
            "app.session.changeDirectory",
            &["alt+w"],
            "Change working directory (move to another repo without quitting)",
        ),
        entry("app.settings.open", &["alt+s"], "Open settings"),
        entry("app.hotkeys.open", &["alt+k"], "Show keyboard shortcuts"),
        // ── Overlays — live only while their surface is open
        entry(
            "app.scroll.lineUp",
            &["up"],
            "Scroll up a line (pinned view)",
        ),
        entry(
            "app.scroll.lineDown",
            &["down"],
            "Scroll down a line (pinned view)",
        ),
        entry(
            "app.scroll.exit",
            &["escape"],
            "Leave the pinned view and follow live output",
        ),
        entry(
            "app.scroll.searchInView",
            &["/"],
            "Search the transcript (pinned view)",
        ),
        entry(
            "app.scroll.searchNext",
            &["n"],
            "Next search match (pinned view)",
        ),
        entry(
            "app.scroll.searchPrevious",
            &["p"],
            "Previous search match (pinned view)",
        ),
        entry(
            "app.panel.nextTab",
            &["right", "tab"],
            "Task panel: next tab",
        ),
        entry(
            "app.panel.prevTab",
            &["left", "shift+tab"],
            "Task panel: previous tab",
        ),
        entry("app.panel.up", &["up"], "Task panel: pick the row above"),
        entry(
            "app.panel.down",
            &["down"],
            "Task panel: pick the row below",
        ),
        entry(
            "app.panel.open",
            &["enter"],
            "Task panel: open the picked row",
        ),
        entry(
            "app.panel.cancel",
            &["x"],
            "Task panel: cancel the picked run (press twice)",
        ),
        entry(
            "app.panel.exit",
            &["escape"],
            "Task panel: back to the prompt",
        ),
        entry(
            "app.subagent.attach",
            &["ctrl+n"],
            "Attach to the subagent run of the last Agent block (read-only)",
        ),
        entry(
            "app.subagent.detach",
            &["escape"],
            "Back from an attached subagent run to the session",
        ),
        entry(
            "app.options.next",
            &["right"],
            "Confirm and advance to the next question",
        ),
        entry(
            "app.options.back",
            &["left"],
            "Go back to the previous question",
        ),
        entry(
            "app.session.togglePath",
            &["alt+p"],
            "Toggle session path display",
        ),
        entry(
            "app.session.toggleSort",
            &["alt+o"],
            "Toggle session sort order",
        ),
        entry(
            "app.session.toggleNamedFilter",
            &["alt+n"],
            "Toggle named session filter",
        ),
        entry("app.session.rename", &["alt+r"], "Rename session"),
        entry("app.session.delete", &["alt+x"], "Delete session"),
        entry(
            "app.session.deleteNoninvasive",
            &["ctrl+backspace"],
            "Delete session when query is empty",
        ),
        entry("app.models.save", &["alt+s"], "Save model selection"),
        entry("app.models.enableAll", &["alt+a"], "Enable all models"),
        entry("app.models.clearAll", &["alt+x"], "Clear all models"),
        entry(
            "app.models.toggleProvider",
            &["alt+g"],
            "Toggle all models for provider",
        ),
        entry(
            "app.models.reorderUp",
            &["alt+up"],
            "Move model up in order",
        ),
        entry(
            "app.models.reorderDown",
            &["alt+down"],
            "Move model down in order",
        ),
        entry(
            "app.tree.foldOrUp",
            &["ctrl+left", "alt+left"],
            "Fold tree branch or move up",
        ),
        entry(
            "app.tree.unfoldOrDown",
            &["ctrl+right", "alt+right"],
            "Unfold tree branch or move down",
        ),
        entry("app.tree.editLabel", &["alt+l"], "Edit tree label"),
        entry(
            "app.tree.toggleLabelTimestamp",
            &["alt+t"],
            "Toggle tree label timestamps",
        ),
        entry(
            "app.tree.filter.default",
            &["alt+1"],
            "Tree filter: default view",
        ),
        entry(
            "app.tree.filter.noTools",
            &["alt+2"],
            "Tree filter: hide tool results",
        ),
        entry(
            "app.tree.filter.userOnly",
            &["alt+3"],
            "Tree filter: user messages only",
        ),
        entry(
            "app.tree.filter.labeledOnly",
            &["alt+4"],
            "Tree filter: labeled entries only",
        ),
        entry(
            "app.tree.filter.all",
            &["alt+5"],
            "Tree filter: show all entries",
        ),
        entry(
            "app.tree.filter.cycleForward",
            &["alt+c"],
            "Tree filter: cycle forward",
        ),
        entry(
            "app.tree.filter.cycleBackward",
            &["shift+alt+c"],
            "Tree filter: cycle backward",
        ),
    ]
}

/// The table `keybindings` returns, built once: it is constant for the
/// process (the only input is the platform).
fn table() -> &'static [KeybindingEntry] {
    static TABLE: OnceLock<Vec<KeybindingEntry>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut all: Vec<KeybindingEntry> = TUI_KEYBINDINGS
            .iter()
            .map(|(id, keys, description)| entry(id, keys, description))
            .collect();
        all.extend(app_keybindings());
        all
    })
}

/// `KEYBINDINGS`: the TUI library's bindings, then the app's, in declaration
/// order.
pub fn keybindings() -> Vec<KeybindingEntry> {
    table().to_vec()
}

/// The definition of one binding, if it exists.
pub fn keybinding(id: &str) -> Option<KeybindingEntry> {
    table().iter().find(|e| e.id == id).cloned()
}

/// [`keybindings`] as the TUI manager's definition map.
pub fn keybinding_definitions() -> HashMap<String, KeybindingDefinition> {
    table()
        .iter()
        .map(|e| {
            (
                e.id.to_string(),
                KeybindingDefinition::new(&e.default_keys, e.description),
            )
        })
        .collect()
}

/// `toKeybindingsConfig`: keep string and string-array bindings; a string
/// becomes a one-key list.
pub fn to_keybindings_config(value: &Value) -> HashMap<String, Vec<String>> {
    let Value::Object(map) = value else {
        return HashMap::new();
    };
    let mut config = HashMap::new();
    for (key, binding) in map {
        match binding {
            Value::String(s) => {
                config.insert(key.clone(), vec![s.clone()]);
            }
            Value::Array(items) if items.iter().all(Value::is_string) => {
                config.insert(
                    key.clone(),
                    items
                        .iter()
                        .map(|v| v.as_str().unwrap_or_default().to_string())
                        .collect(),
                );
            }
            _ => {}
        }
    }
    config
}

/// `orderKeybindingsConfig`: known ids in declaration order, then the rest
/// sorted.
pub fn order_keybindings_config(config: &Map<String, Value>) -> Map<String, Value> {
    let mut ordered = Map::new();
    for e in keybindings() {
        if let Some(v) = config.get(e.id) {
            ordered.insert(e.id.to_string(), v.clone());
        }
    }
    let mut extras: Vec<&String> = config
        .keys()
        .filter(|k| !ordered.contains_key(*k))
        .collect();
    extras.sort();
    for key in extras {
        ordered.insert(key.clone(), config[key].clone());
    }
    ordered
}

/// The app's keybindings manager: the TUI manager over [`keybindings`], plus
/// the parsed `keybindings.json` it was loaded from (leader, timeout, context
/// bindings and warnings).
#[derive(Debug, Clone)]
pub struct AppKeybindingsManager {
    manager: KeybindingsManager,
    config_path: Option<PathBuf>,
    config: KeymapConfig,
}

impl AppKeybindingsManager {
    pub fn new(user_bindings: HashMap<String, Vec<String>>, config_path: Option<PathBuf>) -> Self {
        Self {
            manager: KeybindingsManager::new(keybinding_definitions(), user_bindings),
            config_path,
            config: KeymapConfig::defaults(),
        }
    }

    /// `KeybindingsManager.create`: load `<agent_dir>/keybindings.json`.
    /// A missing file gives the defaults. A bad file gives the defaults and a warning.
    pub fn create(agent_dir: Option<&Path>) -> Self {
        let agent_dir = agent_dir
            .map(Path::to_path_buf)
            .unwrap_or_else(hoocode_code_paths::agent_dir);
        let config_path = agent_dir.join("keybindings.json");
        let mut manager = Self::new(HashMap::new(), Some(config_path.clone()));
        manager.apply(KeymapConfig::load(&config_path));
        manager
    }

    /// Re-read the config file. A file that is now missing gives the defaults.
    pub fn reload(&mut self) {
        if let Some(path) = self.config_path.clone() {
            self.apply(KeymapConfig::load(&path));
        }
    }

    /// Takes a parsed file (`None`: no file, so the defaults).
    fn apply(&mut self, loaded: Option<KeymapConfig>) {
        let mut config = loaded.unwrap_or_else(KeymapConfig::defaults);
        for action in config.flat.keys() {
            if keybinding(action).is_none() {
                config
                    .warnings
                    .push(format!("unknown action {action:?}; skipped"));
            }
        }
        for o in &config.overrides {
            if let Some(action) = &o.action {
                if keybinding(action).is_none() {
                    config.warnings.push(format!(
                        "{}: unknown action {action:?}; skipped",
                        o.context.name()
                    ));
                }
            }
        }
        self.manager.set_user_bindings(config.flat.clone());
        self.config = config;
    }

    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// The leader key (`ctrl+x` unless the file names another).
    pub fn leader(&self) -> &LeaderKey {
        &self.config.leader
    }

    /// How long a pending leader waits for its next key.
    pub fn leader_timeout(&self) -> Duration {
        self.config.leader_timeout
    }

    /// Problems found in the file. Each names what was skipped.
    pub fn warnings(&self) -> &[String] {
        &self.config.warnings
    }

    /// The grouped form's bindings, and the flat form's sequences, as read.
    pub fn overrides(&self) -> &[ContextOverride] {
        &self.config.overrides
    }

    /// The action bound to `key` in `context`, or `None` when the key is
    /// unbound there. A key the file sets in this context (or in `Global`)
    /// wins. Any other key uses the flat bindings, defaults included. If
    /// several actions share a key, the first id in sorted order wins.
    pub fn action_for_key(&self, context: Context, key: &str) -> Option<String> {
        let key = normalize_key(key);
        if let Some(action) = effective_overrides(&self.config.overrides, context).get(&key) {
            return action.clone();
        }
        let resolved = self.manager.get_resolved_bindings();
        let mut ids: Vec<&String> = resolved
            .iter()
            .filter(|(_, keys)| keys.iter().any(|k| normalize_key(k) == key))
            .map(|(id, _)| id)
            .collect();
        ids.sort();
        ids.first().map(|id| (*id).clone())
    }

    /// A leader state machine for `context`. It binds every multi-step
    /// sequence that applies there: `Global`, the `Leader` context (written
    /// after the leader), and `context` itself. Unbound sequences are left out.
    pub fn leader_machine(&self, context: Context) -> LeaderMachine {
        let mut machine =
            LeaderMachine::new(self.config.leader.clone(), self.config.leader_timeout);
        let leader = self.config.leader.id();
        let mut table: BTreeMap<String, Option<String>> = BTreeMap::new();
        for o in &self.config.overrides {
            let full = match o.context {
                Context::Global => o.key.clone(),
                Context::Leader => normalize_key(&format!("{leader} {}", o.key)),
                _ if o.context == context => o.key.clone(),
                _ => continue,
            };
            table.insert(full, o.action.clone());
        }
        for (key, action) in table {
            let Some(action) = action else { continue };
            if key.split_whitespace().count() > 1 {
                // The parser checked that each sequence starts with the leader
                // and uses no reserved step, so bind cannot fail here.
                let _ = machine.bind(&key, &action);
            }
        }
        machine
    }

    /// Install as the manager library components consult.
    pub fn install(&self) {
        hoocode_tui_keys::set_keybindings(self.manager.clone());
    }

    pub fn into_manager(self) -> KeybindingsManager {
        self.manager
    }
}

impl Default for AppKeybindingsManager {
    fn default() -> Self {
        Self::new(HashMap::new(), None)
    }
}

impl Deref for AppKeybindingsManager {
    type Target = KeybindingsManager;
    fn deref(&self) -> &KeybindingsManager {
        &self.manager
    }
}

impl DerefMut for AppKeybindingsManager {
    fn deref_mut(&mut self) -> &mut KeybindingsManager {
        &mut self.manager
    }
}
