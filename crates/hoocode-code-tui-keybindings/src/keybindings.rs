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
    KeySequence, KeybindingDefinition, KeybindingsManager, LeaderKey, LeaderMachine,
    TUI_KEYBINDINGS,
};
use serde_json::{Map, Value};

use crate::config::{
    effective_overrides, normalize_key, Context, ContextOverride, KeymapConfig, DEFAULT_LEADER,
};
use crate::defaults::context_of;

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
        // ── Quit — leaving with jobs running
        entry("app.quit.stop-jobs", &["k"], "Stop jobs and quit"),
        entry("app.quit.leave-running", &["l"], "Leave jobs running"),
        // ── Compose — the message in your hands
        entry("app.editor.external", &["ctrl+x e"], "Open external editor"),
        entry(
            "app.input.voiceTranscribe",
            &["ctrl+x r"],
            "Record voice and transcribe into the editor",
        ),
        entry(
            "app.clipboard.pasteImage",
            if windows { &["ctrl+x v"] } else { &["ctrl+v"] },
            "Paste image from clipboard",
        ),
        entry(
            "app.clipboard.copyMessage",
            &["ctrl+x c"],
            "Copy the agent's last message to the clipboard",
        ),
        entry(
            "app.message.followUp",
            &["ctrl+x f"],
            "Queue follow-up message",
        ),
        entry(
            "app.message.dequeue",
            &["ctrl+x d"],
            "Take back the last queued message",
        ),
        entry(
            "app.history.search",
            &["ctrl+r"],
            "Search prompt history (reverse, incremental)",
        ),
        // ── Steer — what the agent is before it runs
        entry(
            "app.mode.cycleForward",
            &["shift+tab"],
            "Cycle agent mode (ask → plan → build → debug)",
        ),
        entry(
            "app.mode.cycleBackward",
            &["ctrl+x a"],
            "Cycle agent mode backward",
        ),
        entry(
            "app.model.cycleForward",
            &["ctrl+x m"],
            "Cycle to next model",
        ),
        entry(
            "app.model.cycleBackward",
            &["ctrl+x M"],
            "Cycle to previous model",
        ),
        entry("app.model.select", &[], "Open model selector"),
        entry(
            "app.thinking.cycleForward",
            &["ctrl+x t"],
            "Cycle thinking level (off → … → high)",
        ),
        entry(
            "app.thinking.cycleBackward",
            &["ctrl+x T"],
            "Cycle thinking level backward",
        ),
        entry(
            "app.thinking.toggle",
            &["ctrl+x b"],
            "Toggle thinking blocks",
        ),
        entry(
            "app.tasks.cycleForward",
            &["ctrl+x l"],
            "Cycle task panel tab (Plan → Agents → Shell → Schedule → MCP)",
        ),
        entry(
            "app.tasks.cycleBackward",
            &["ctrl+x L"],
            "Cycle task panel view backward",
        ),
        entry(
            "app.panel.focus",
            &["ctrl+x g"],
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
            &["home"],
            "Jump to the start of the transcript",
        ),
        entry("app.scroll.bottom", &["end"], "Jump back to live output"),
        entry("app.scroll.search", &["ctrl+x /"], "Search the transcript"),
        entry(
            "app.scroll.previousMessage",
            &["["],
            "Jump to your previous message",
        ),
        entry(
            "app.scroll.nextMessage",
            &["]"],
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
            &["ctrl+x h"],
            "Resume a session from history",
        ),
        entry("app.session.tree", &[], "Open session tree"),
        entry("app.session.new", &[], "Start a new session"),
        entry("app.session.fork", &[], "Fork current session"),
        entry(
            "app.session.changeDirectory",
            &["ctrl+x w"],
            "Change working directory (move to another repo without quitting)",
        ),
        entry("app.settings.open", &["ctrl+x s"], "Open settings"),
        entry(
            "app.hotkeys.open",
            &["ctrl+x ?", "ctrl+x k"],
            "Show keyboard shortcuts",
        ),
        // ── Overlays — live only while their surface is open
        entry(
            "app.scroll.lineUp",
            &["up", "k"],
            "Scroll up a line (pinned view)",
        ),
        entry(
            "app.scroll.lineDown",
            &["down", "j"],
            "Scroll down a line (pinned view)",
        ),
        entry(
            "app.scroll.exit",
            &["escape", "q"],
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
            "app.scroll.focusPrevious",
            &["{"],
            "Focus the previous tool block (pinned view)",
        ),
        entry(
            "app.scroll.focusNext",
            &["}"],
            "Focus the next tool block (pinned view)",
        ),
        entry(
            "app.scroll.expandFocused",
            &["enter"],
            "Toggle the focused tool block (pinned view)",
        ),
        entry(
            "app.scroll.copyFocused",
            &["y"],
            "Copy the focused block's raw output (pinned view)",
        ),
        entry(
            "app.scroll.focusNewest",
            &["ctrl+x u"],
            "Focus the newest tool block and pin the view",
        ),
        entry(
            "app.scroll.toggleAll",
            &["ctrl+x o"],
            "Open all tool blocks, or close them all when one is open",
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
            &["ctrl+x n"],
            "Attach to the subagent run of the last Agent block (read-only)",
        ),
        entry(
            "app.subagent.detach",
            &["escape"],
            "Back from an attached subagent run to the session",
        ),
        entry(
            "app.subagent.view",
            &["v"],
            "Interrupted run: view the transcript",
        ),
        entry("app.subagent.resume", &["r"], "Interrupted run: resume it"),
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
            &["p"],
            "Toggle session path display",
        ),
        entry(
            "app.session.toggleSort",
            &["o"],
            "Toggle session sort order",
        ),
        entry(
            "app.session.toggleNamedFilter",
            &["n"],
            "Toggle named session filter",
        ),
        entry("app.session.rename", &["r"], "Rename session"),
        entry("app.session.delete", &["x"], "Delete session"),
        entry(
            "app.session.deleteNoninvasive",
            &["ctrl+backspace"],
            "Delete session when query is empty",
        ),
        entry("app.models.save", &["s"], "Save model selection"),
        entry("app.models.enableAll", &["a"], "Enable all models"),
        entry("app.models.clearAll", &["x"], "Clear all models"),
        entry(
            "app.models.toggleProvider",
            &["g"],
            "Toggle all models for provider",
        ),
        entry(
            "app.models.reorderUp",
            &["shift+up"],
            "Move model up in order",
        ),
        entry(
            "app.models.reorderDown",
            &["shift+down"],
            "Move model down in order",
        ),
        entry("app.models.category", &["c"], "Model: category"),
        entry(
            "app.tree.foldOrUp",
            &["h", "left"],
            "Fold tree branch or move up",
        ),
        entry(
            "app.tree.unfoldOrDown",
            &["l", "right"],
            "Unfold tree branch or move down",
        ),
        entry("app.tree.editLabel", &["e"], "Edit tree label"),
        entry(
            "app.tree.toggleLabelTimestamp",
            &["t"],
            "Toggle tree label timestamps",
        ),
        entry(
            "app.tree.filter.default",
            &["1"],
            "Tree filter: default view",
        ),
        entry(
            "app.tree.filter.noTools",
            &["2"],
            "Tree filter: hide tool results",
        ),
        entry(
            "app.tree.filter.userOnly",
            &["3"],
            "Tree filter: user messages only",
        ),
        entry(
            "app.tree.filter.labeledOnly",
            &["4"],
            "Tree filter: labeled entries only",
        ),
        entry(
            "app.tree.filter.all",
            &["5"],
            "Tree filter: show all entries",
        ),
        entry(
            "app.tree.filter.cycleForward",
            &["c"],
            "Tree filter: cycle forward",
        ),
        entry(
            "app.tree.filter.cycleBackward",
            &[],
            "Tree filter: cycle backward",
        ),
        entry(
            "app.settings.toggle",
            &["space"],
            "Settings: toggle the picked row",
        ),
        // ── Lists — bare letters, live while the query is empty (K-P4)
        entry("app.list.up", &["k"], "List: move up"),
        entry("app.list.down", &["j"], "List: move down"),
        entry("app.list.search", &["/"], "List: focus the search"),
        entry("app.list.help", &["f1", "?"], "List: show or hide the keys"),
        entry("app.list.close", &["q"], "List: close"),
        // ── Dialogs — letters answer while no reply is typed (K-P4)
        entry("app.dialog.yes", &["y"], "Dialog: yes"),
        entry("app.dialog.no", &["n"], "Dialog: no"),
        entry("app.dialog.always", &["a"], "Dialog: always"),
        // ── Permission — the clearance card's own keys (K-P4, bare while no reply is typed)
        entry(
            "app.permission.edit",
            &["e"],
            "Permission: edit the command first",
        ),
        entry("app.permission.pick1", &["1"], "Permission: pick option 1"),
        entry("app.permission.pick2", &["2"], "Permission: pick option 2"),
        entry("app.permission.pick3", &["3"], "Permission: pick option 3"),
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
    keybinding_definitions_for(DEFAULT_LEADER)
}

/// The definition map with the default leader sequences written with `leader`
/// (`"ctrl+x m"` becomes `"<leader> m"`). Other keys are unchanged.
pub fn keybinding_definitions_for(leader: &str) -> HashMap<String, KeybindingDefinition> {
    table()
        .iter()
        .map(|e| {
            let keys: Vec<String> = e
                .default_keys
                .iter()
                .map(|key| with_leader(key, leader))
                .collect();
            let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
            (
                e.id.to_string(),
                KeybindingDefinition::new(&refs, e.description),
            )
        })
        .collect()
}

/// Swaps the default leader in a default key for `leader`. A key that does not
/// start with the default leader is returned as it is.
fn with_leader(key: &str, leader: &str) -> String {
    match key.strip_prefix(DEFAULT_LEADER) {
        Some(rest) if rest.starts_with(' ') => format!("{leader}{rest}"),
        _ => key.to_string(),
    }
}

/// One leader sequence that applies in a context: the key typed after the
/// leader, the action it runs, and the action's description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderEntry {
    /// The keys after the leader, for example `"m"`.
    pub key: String,
    pub action: String,
    pub description: String,
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

    /// The manager for a parsed `keybindings.json`, with no file on disk. For
    /// tests and tools that hold the text already.
    pub fn from_json_str(text: &str) -> Self {
        let mut manager = Self::new(HashMap::new(), None);
        manager.apply(Some(KeymapConfig::from_json_str(text)));
        manager
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
        // The default leader sequences follow the configured leader. The
        // single keys of the grouped form are folded into each action's keys,
        // so the screens that ask "is this key bound to X?" see them too.
        let definitions = keybinding_definitions_for(config.leader.id());
        let resolved = KeybindingsManager::new(definitions.clone(), config.flat.clone())
            .get_resolved_bindings();
        config
            .warnings
            .extend(leader_shadow_warnings(config.leader.id()));
        config
            .warnings
            .extend(flat_default_collisions(&resolved, &config.flat));
        let keys = keys_after_overrides(resolved, &config.overrides, &mut config.warnings);
        self.manager = KeybindingsManager::new(definitions, keys);
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

    /// Bindings kept but cautioned about: a key the terminal or the shell often takes.
    pub fn cautions(&self) -> &[String] {
        &self.config.cautions
    }

    /// Every key the file lists (flat values, grouped keys, leader sequences),
    /// sorted and without repeats. The Alt notice reads it.
    pub fn bound_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self
            .config
            .flat
            .values()
            .flatten()
            .cloned()
            .chain(self.config.overrides.iter().map(|o| o.key.clone()))
            .collect();
        keys.sort();
        keys.dedup();
        keys
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

    /// The leader sequences that apply in `context`: the defaults, then the
    /// file's entries for `Global`, `Leader` (written after the leader) and
    /// `context`. A later entry for the same key wins, and `null` removes it.
    /// Maps each full key (`"ctrl+x m"`) to its action.
    pub fn leader_table(&self, context: Context) -> BTreeMap<String, String> {
        let leader = self.config.leader.id();
        let mut table: BTreeMap<String, Option<String>> = BTreeMap::new();
        for (id, keys) in self.manager.get_resolved_bindings() {
            for key in keys {
                let key = normalize_key(&key);
                if is_leader_sequence(&key, leader) {
                    table.insert(key, Some(id.clone()));
                }
            }
        }
        for o in &self.config.overrides {
            let full = match o.context {
                Context::Global => o.key.clone(),
                Context::Leader => normalize_key(&format!("{leader} {}", o.key)),
                _ if o.context == context => o.key.clone(),
                _ => continue,
            };
            table.insert(full, o.action.clone());
        }
        table
            .into_iter()
            .filter_map(|(key, action)| action.map(|action| (key, action)))
            .collect()
    }

    /// The leader sequences in the prompt, as the which-key popup lists them:
    /// the key after the leader, the action, and its description. Sorted by key.
    pub fn leader_entries(&self) -> Vec<LeaderEntry> {
        let leader_len = self.config.leader.id().len();
        self.leader_table(Context::Prompt)
            .into_iter()
            .map(|(full, action)| {
                let key = full
                    .get(leader_len..)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let description = keybinding(&action)
                    .map(|e| e.description.to_string())
                    .unwrap_or_else(|| action.clone());
                LeaderEntry {
                    key,
                    action,
                    description,
                }
            })
            .collect()
    }

    /// A leader state machine for `context`, bound to [`Self::leader_table`].
    pub fn leader_machine(&self, context: Context) -> LeaderMachine {
        let mut machine =
            LeaderMachine::new(self.config.leader.clone(), self.config.leader_timeout);
        for (key, action) in self.leader_table(context) {
            // Each key starts with the leader and uses no reserved step (the
            // file parser checked that), so bind cannot fail here.
            let _ = machine.bind(&key, &action);
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

/// Whether a grouped binding's key is a single key (not a leader sequence and
/// not a `Leader`-context key, which the leader table handles).
fn is_single_key(o: &ContextOverride) -> bool {
    o.context != Context::Leader && !is_multi_step(&o.key)
}

/// Whether a key text is a sequence of two or more steps.
fn is_multi_step(key: &str) -> bool {
    KeySequence::parse(key).is_ok_and(|s| s.steps().len() > 1)
}

/// The key lists of every action after the file's single-key grouped bindings.
///
/// A binding applies in the context of its action: `Global` applies to every
/// action, and another context applies to the actions defined in it (see
/// `defaults::context_of`). A binding of `key` to an action adds the key to
/// that action and takes it from the other actions of the binding's context
/// (for `Global`, the Global actions only). `null` or `"none"` takes the key
/// from every action in the scope, `Global` included. A binding whose
/// action runs in another context is skipped with a warning. `Global` entries
/// are applied first, so a context entry for the same key wins.
fn keys_after_overrides(
    mut keys: HashMap<String, Vec<String>>,
    overrides: &[ContextOverride],
    warnings: &mut Vec<String>,
) -> HashMap<String, Vec<String>> {
    let mut singles: Vec<&ContextOverride> =
        overrides.iter().filter(|o| is_single_key(o)).collect();
    singles.sort_by_key(|o| o.context != Context::Global);
    for o in singles {
        let key = normalize_key(&o.key);
        let in_scope = |id: &str| o.context == Context::Global || context_of(id) == o.context;
        match o.action.as_deref() {
            None => {
                for (id, list) in keys.iter_mut() {
                    if in_scope(id) {
                        list.retain(|k| normalize_key(k) != key);
                    }
                }
            }
            Some(action) => {
                if keybinding(action).is_none() {
                    // Reported as an unknown action when the file is read.
                    continue;
                }
                if !in_scope(action) {
                    warnings.push(format!(
                        "{}: {key} is bound to {action}, which runs in the {} context; skipped",
                        o.context.name(),
                        context_of(action).name()
                    ));
                    continue;
                }
                // Taking a key removes it from the actions of the binding's own
                // context only. A Global binding takes it from the Global actions
                // alone, so it never strips a key from another context.
                let takes_from = |id: &str| context_of(id) == o.context;
                for (id, list) in keys.iter_mut() {
                    if id == action {
                        if !list.iter().any(|k| normalize_key(k) == key) {
                            list.push(key.clone());
                        }
                    } else if takes_from(id) {
                        list.retain(|k| normalize_key(k) != key);
                    }
                }
            }
        }
    }
    keys
}

/// The result of `hoocode keybindings check`: the file's problems. `errors`
/// are bindings skipped or ignored, and `cautions` are bindings kept that may not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// The file that was read (`<agent dir>/keybindings.json`).
    pub path: PathBuf,
    /// Whether the file exists. A missing file is fine: the defaults apply.
    pub exists: bool,
    pub errors: Vec<String>,
    pub cautions: Vec<String>,
}

/// Reads `<agent_dir>/keybindings.json` and reports what it found. Never fails.
pub fn check_file(agent_dir: &Path) -> CheckReport {
    let manager = AppKeybindingsManager::create(Some(agent_dir));
    let path = agent_dir.join("keybindings.json");
    CheckReport {
        exists: path.is_file(),
        path,
        errors: manager.warnings().to_vec(),
        cautions: manager.cautions().to_vec(),
    }
}

/// Whether two actions can meet on one key: they share a context, or one of
/// them is `Global`. A key is not a collision across two other contexts.
fn same_scope(a: &str, b: &str) -> bool {
    let (ca, cb) = (context_of(a), context_of(b));
    ca == cb || ca == Context::Global || cb == Context::Global
}

/// A flat binding of a key that an app action uses by default, in the same
/// scope. Both keep the key, so the warning names both. The library's editor
/// keys (`tui.*`) are not checked: rebinding one of them is deliberate and
/// common. The flat form of two actions that both change is reported by the
/// file reader instead.
fn flat_default_collisions(
    resolved: &HashMap<String, Vec<String>>,
    flat: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut owners: Vec<&String> = flat.keys().collect();
    owners.sort();
    for action in owners {
        for key in flat[action].iter().map(|k| normalize_key(k).to_lowercase()) {
            if key.contains(' ') {
                continue;
            }
            let mut others: Vec<&String> = resolved
                .iter()
                .filter(|(id, keys)| {
                    id.as_str() != action
                        && id.starts_with("app.")
                        && !flat.contains_key(id.as_str())
                        && same_scope(action, id)
                        && keys.iter().any(|k| normalize_key(k).to_lowercase() == key)
                })
                .map(|(id, _)| id)
                .collect();
            others.sort();
            for other in others {
                warnings.push(format!(
                    "{key}: bound to {action} in the file and to {other} by default; both keep it"
                ));
            }
        }
    }
    warnings
}

/// A leader that is also a default single key: the leader runs first, so that
/// binding can no longer be reached by its key.
fn leader_shadow_warnings(leader: &str) -> Vec<String> {
    let leader = normalize_key(leader).to_lowercase();
    table()
        .iter()
        .filter(|e| {
            e.default_keys
                .iter()
                .any(|k| !k.contains(' ') && normalize_key(k).to_lowercase() == leader)
        })
        .map(|e| {
            format!(
                "leader {leader} shadows the default binding of {}; that key now starts the leader",
                e.id
            )
        })
        .collect()
}

/// Whether a whole key is a sequence that starts with `leader`.
fn is_leader_sequence(key: &str, leader: &str) -> bool {
    let mut steps = key.split_whitespace();
    steps
        .next()
        .is_some_and(|first| first.eq_ignore_ascii_case(leader))
        && steps.next().is_some()
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
