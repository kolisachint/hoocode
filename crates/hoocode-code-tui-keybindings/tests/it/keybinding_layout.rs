//! Port of the pin's `test/keybinding-layout.test.ts`: the cockpit layout
//! rules described in `keybindings.rs`.

use std::collections::HashSet;

use hoocode_code_tui_keybindings::*;
use regex::Regex;

/// Live while the prompt editor has focus — the main editor plus every global
/// action.
const GLOBAL_SCOPE: &[&str] = &[
    "app.subagent.attach",
    "tui.editor.cursorUp",
    "tui.editor.cursorDown",
    "tui.editor.historyPrevious",
    "tui.editor.historyNext",
    "tui.editor.cursorLeft",
    "tui.editor.cursorRight",
    "tui.editor.cursorWordLeft",
    "tui.editor.cursorWordRight",
    "tui.editor.cursorLineStart",
    "tui.editor.cursorLineEnd",
    "tui.editor.jumpForward",
    "tui.editor.jumpBackward",
    "tui.editor.pageUp",
    "tui.editor.pageDown",
    "tui.editor.deleteCharBackward",
    "tui.editor.deleteWordBackward",
    "tui.editor.deleteWordForward",
    "tui.editor.deleteToLineStart",
    "tui.editor.deleteToLineEnd",
    "tui.editor.yank",
    "tui.editor.yankPop",
    "tui.editor.undo",
    "tui.editor.redo",
    "tui.input.newLine",
    "tui.input.submit",
    "tui.input.tab",
    "app.interrupt",
    "app.suspend",
    "app.thinking.cycleForward",
    "app.thinking.cycleBackward",
    "app.model.cycleForward",
    "app.model.cycleBackward",
    "app.model.select",
    "app.tools.expand",
    "app.view.cycleForward",
    "app.view.cycleBackward",
    "app.thinking.toggle",
    "app.scroll.pageUp",
    "app.scroll.pageDown",
    "app.scroll.top",
    "app.scroll.bottom",
    "app.scroll.previousMessage",
    "app.scroll.nextMessage",
    "app.scroll.search",
    "app.tasks.cycleForward",
    "app.tasks.cycleBackward",
    "app.panel.focus",
    "app.editor.external",
    "app.input.voiceTranscribe",
    "app.message.followUp",
    "app.message.dequeue",
    "app.scroll.focusNewest",
    "app.history.search",
    "app.clipboard.copyMessage",
    "app.clipboard.pasteImage",
    "app.session.new",
    "app.session.tree",
    "app.session.fork",
    "app.session.resume",
    "app.session.changeDirectory",
    "app.settings.open",
    "app.hotkeys.open",
    "app.mode.cycleForward",
    "app.mode.cycleBackward",
];

const SESSION_PICKER_SCOPE: &[&str] = &[
    "tui.select.up",
    "tui.select.down",
    "tui.select.pageUp",
    "tui.select.pageDown",
    "tui.select.confirm",
    "app.session.togglePath",
    "app.session.toggleSort",
    "app.session.rename",
    "app.session.delete",
    "app.session.deleteNoninvasive",
    "app.session.toggleNamedFilter",
    "app.list.up",
    "app.list.down",
    "app.list.search",
    "app.list.help",
    "app.list.close",
];

const MODELS_PICKER_SCOPE: &[&str] = &[
    "tui.select.up",
    "tui.select.down",
    "tui.select.confirm",
    "app.models.save",
    "app.models.enableAll",
    "app.models.clearAll",
    "app.models.toggleProvider",
    "app.models.reorderUp",
    "app.models.reorderDown",
    "app.models.category",
    "app.list.up",
    "app.list.down",
    "app.list.search",
    "app.list.help",
    "app.list.close",
];

const TREE_SCOPE: &[&str] = &[
    "tui.select.up",
    "tui.select.down",
    "app.tree.foldOrUp",
    "app.tree.unfoldOrDown",
    "app.tree.filter.default",
    "app.tree.filter.noTools",
    "app.tree.filter.userOnly",
    "app.tree.filter.labeledOnly",
    "app.tree.filter.all",
    "app.tree.filter.cycleForward",
    "app.tree.filter.cycleBackward",
    "app.tree.editLabel",
    "app.tree.toggleLabelTimestamp",
    "tui.select.confirm",
    "tui.select.cancel",
    "tui.select.pageUp",
    "tui.select.pageDown",
    "tui.editor.deleteCharBackward",
    "app.list.up",
    "app.list.down",
    "app.list.search",
    "app.list.help",
    "app.list.close",
];

/// The generic selector (approval and confirm dialogs): letters answer while
/// no reply is typed (K-P4), and the list keys work as in a picker.
const DIALOG_SCOPE: &[&str] = &[
    "tui.select.up",
    "tui.select.down",
    "tui.select.confirm",
    "tui.select.cancel",
    "app.list.up",
    "app.list.down",
    "app.list.help",
    "app.list.close",
    "app.dialog.yes",
    "app.dialog.no",
    "app.dialog.always",
];

/// Live while the task panel has focus. It takes the keyboard then, so keys it
/// shares with the prompt (`tab`, `up`, `enter`) are fine here.
const PANEL_SCOPE: &[&str] = &[
    "app.panel.focus",
    "app.panel.nextTab",
    "app.panel.prevTab",
    "app.panel.up",
    "app.panel.down",
    "app.panel.open",
    "app.panel.cancel",
    "app.panel.exit",
];

/// The attached subagent view (T3.3): Esc goes back, nothing else is live.
const ATTACHED_SCOPE: &[&str] = &["app.subagent.detach"];

const OPTIONS_SCOPE: &[&str] = &[
    "tui.select.up",
    "tui.select.down",
    "tui.select.confirm",
    "tui.select.cancel",
    "app.options.next",
    "app.options.back",
];

const SCROLL_SCOPE: &[&str] = &[
    "app.scroll.lineUp",
    "app.scroll.lineDown",
    "app.scroll.previousMessage",
    "app.scroll.nextMessage",
    "app.scroll.shiftLineUp",
    "app.scroll.shiftLineDown",
    "app.scroll.pageUp",
    "app.scroll.pageDown",
    "app.scroll.top",
    "app.scroll.bottom",
    "app.scroll.exit",
    "app.scroll.search",
    "app.scroll.searchInView",
    "app.scroll.searchNext",
    "app.scroll.searchPrevious",
    "app.scroll.focusPrevious",
    "app.scroll.focusNext",
    "app.scroll.expandFocused",
    "app.scroll.copyFocused",
];

const SCOPES: &[(&str, &[&str])] = &[
    ("global", GLOBAL_SCOPE),
    ("session picker", SESSION_PICKER_SCOPE),
    ("models picker", MODELS_PICKER_SCOPE),
    ("session tree", TREE_SCOPE),
    ("dialog selector", DIALOG_SCOPE),
    ("task panel", PANEL_SCOPE),
    ("attached view", ATTACHED_SCOPE),
    ("options pane", OPTIONS_SCOPE),
    ("pinned scroll view", SCROLL_SCOPE),
];

/// Ids no scope lists, each because it shares a key with another on purpose.
const DELIBERATELY_UNSCOPED: &[&str] = &[
    "tui.editor.deleteCharForward",
    "tui.input.copy",
    "app.exit",
    "app.clear",
];

fn manager() -> AppKeybindingsManager {
    AppKeybindingsManager::default()
}

fn ids() -> Vec<&'static str> {
    keybindings().into_iter().map(|e| e.id).collect()
}

fn collisions_within(ids: &[&str]) -> Vec<String> {
    let m = manager();
    let mut by_key: Vec<(String, Vec<&str>)> = Vec::new();
    for id in ids {
        for key in m.get_keys(id) {
            match by_key.iter_mut().find(|(k, _)| *k == key) {
                Some((_, owners)) => owners.push(id),
                None => by_key.push((key, vec![id])),
            }
        }
    }
    by_key
        .into_iter()
        .filter(|(_, owners)| owners.len() > 1)
        .map(|(key, owners)| format!("{key}: {}", owners.join(" + ")))
        .collect()
}

/// The bytes a terminal without the Kitty protocol sends for a key.
fn legacy_input(key: &str) -> Option<String> {
    if Regex::new(r"^alt\+[a-z0-9]$").unwrap().is_match(key) {
        return Some(format!("\x1b{}", &key[4..]));
    }
    if Regex::new(r"^ctrl\+[a-z]$").unwrap().is_match(key) {
        let c = key.as_bytes()[5] - 96;
        return Some((c as char).to_string());
    }
    None
}

fn legacy_collisions_within(ids: &[&str]) -> Vec<String> {
    let m = manager();
    let mut inputs: Vec<String> = Vec::new();
    for id in ids {
        for key in m.get_keys(id) {
            if let Some(input) = legacy_input(&key) {
                if !inputs.contains(&input) {
                    inputs.push(input);
                }
            }
        }
    }
    let mut collisions = Vec::new();
    for input in inputs {
        let owners: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| m.matches(&input, id))
            .collect();
        if owners.len() > 1 {
            collisions.push(format!("{input:?}: {}", owners.join(" + ")));
        }
    }
    collisions
}

fn has_alt(key: &str) -> bool {
    key.split('+').any(|p| p == "alt")
}

#[test]
fn has_no_two_bindings_on_the_same_key_in_any_scope() {
    for (name, ids) in SCOPES {
        assert_eq!(collisions_within(ids), Vec::<String>::new(), "{name}");
    }
}

#[test]
fn has_no_two_bindings_a_single_legacy_keystroke_fires_in_any_scope() {
    for (name, ids) in SCOPES {
        assert_eq!(
            legacy_collisions_within(ids),
            Vec::<String>::new(),
            "{name}"
        );
    }
}

#[test]
fn checks_every_binding_in_some_scope() {
    let scoped: HashSet<&str> = SCOPES
        .iter()
        .flat_map(|(_, ids)| ids.iter().copied())
        .collect();
    let unchecked: Vec<&str> = ids()
        .into_iter()
        .filter(|id| !scoped.contains(id) && !DELIBERATELY_UNSCOPED.contains(id))
        .collect();
    assert_eq!(unchecked, Vec::<&str>::new());
}

/// A key a terminal without the Kitty protocol can actually send. Shift+tab
/// is the one shifted key every terminal sends. Shift+up and shift+down are
/// the xterm form of the arrows (`ESC [ 1 ; 2 A`/`B`), which terminals send
/// without Kitty too.
fn reachable_without_kitty(key: &str) -> bool {
    !key.starts_with("shift+") || matches!(key, "shift+tab" | "shift+up" | "shift+down")
}

#[test]
fn gives_every_action_at_least_one_key_a_non_kitty_terminal_can_send() {
    let m = manager();
    let unreachable: Vec<&str> = ids()
        .into_iter()
        .filter(|id| {
            let keys = m.get_keys(id);
            !keys.is_empty() && !keys.iter().any(|k| reachable_without_kitty(k))
        })
        .collect();
    // Every default is reachable: a leader sequence starts with Ctrl+X and
    // its second key is a plain key (a capital is typed as itself).
    assert_eq!(unreachable, Vec::<&str>::new());
}

#[test]
fn binds_no_verb_to_a_bare_shift_letter() {
    let m = manager();
    let re = Regex::new(r"^shift\+[a-z]$").unwrap();
    let mut shift_letters = Vec::new();
    for id in ids() {
        for key in m.get_keys(id) {
            if re.is_match(&key) {
                shift_letters.push(format!("{id}: {key}"));
            }
        }
    }
    assert_eq!(shift_letters, Vec::<String>::new());
}

#[test]
fn binds_no_default_to_an_alt_key() {
    // No Alt or Option key in any default (K-P1). Alt keys a user writes in
    // keybindings.json still work (see `keymap_config`).
    let m = manager();
    let with_alt: Vec<String> = ids()
        .into_iter()
        .filter(|id| m.get_keys(id).iter().any(|k| has_alt(k)))
        .map(str::to_string)
        .collect();
    assert_eq!(with_alt, Vec::<String>::new());
}

#[test]
fn binds_ctrl_r_only_to_prompt_history_search() {
    // Ctrl+R is prompt-history search (K-7). Transcript search is leader `/`.
    let m = manager();
    let on_ctrl_r: Vec<&str> = ids()
        .into_iter()
        .filter(|id| m.get_keys(id).iter().any(|k| k == "ctrl+r"))
        .collect();
    assert_eq!(on_ctrl_r, vec!["app.history.search"]);
}

#[test]
fn binds_every_leader_default_as_the_leader_then_one_key() {
    let m = manager();
    let mut bad = Vec::new();
    for id in ids() {
        for key in m.get_keys(id) {
            if key.contains(' ') {
                let steps: Vec<&str> = key.split_whitespace().collect();
                if steps.len() != 2 || steps[0] != "ctrl+x" {
                    bad.push(format!("{id}: {key}"));
                }
            }
        }
    }
    assert_eq!(bad, Vec::<String>::new());
}

/// The plan's default keymap (docs/design/themes-keybindings.md, "Default
/// keymap"), for every key except the picker and scroll-mode keys (K-6).
#[test]
fn matches_the_plans_default_keymap() {
    let m = manager();
    let plan: Vec<(&str, Vec<&str>)> = vec![
        ("app.interrupt", vec!["escape"]),
        ("app.clear", vec!["ctrl+c"]),
        ("app.exit", vec!["ctrl+d"]),
        (
            "app.suspend",
            if cfg!(windows) {
                vec![]
            } else {
                vec!["ctrl+z"]
            },
        ),
        ("app.editor.external", vec!["ctrl+x e"]),
        ("app.input.voiceTranscribe", vec!["ctrl+x r"]),
        (
            "app.clipboard.pasteImage",
            if cfg!(windows) {
                vec!["ctrl+x v"]
            } else {
                vec!["ctrl+v"]
            },
        ),
        ("app.clipboard.copyMessage", vec!["ctrl+x c"]),
        ("app.message.followUp", vec!["ctrl+x f"]),
        ("app.message.dequeue", vec!["ctrl+x d"]),
        ("app.history.search", vec!["ctrl+r"]),
        ("app.mode.cycleForward", vec!["shift+tab"]),
        ("app.mode.cycleBackward", vec!["ctrl+x a"]),
        ("app.model.cycleForward", vec!["ctrl+x m"]),
        ("app.model.cycleBackward", vec!["ctrl+x M"]),
        ("app.thinking.cycleForward", vec!["ctrl+x t"]),
        ("app.thinking.cycleBackward", vec!["ctrl+x T"]),
        ("app.view.cycleForward", vec!["ctrl+x o"]),
        ("app.view.cycleBackward", vec!["ctrl+x O"]),
        ("app.tools.expand", vec!["ctrl+x p"]),
        ("app.thinking.toggle", vec!["ctrl+x b"]),
        ("app.tasks.cycleForward", vec!["ctrl+x l"]),
        ("app.tasks.cycleBackward", vec!["ctrl+x L"]),
        ("app.panel.focus", vec!["ctrl+x g"]),
        ("app.scroll.search", vec!["ctrl+x /"]),
        ("app.session.resume", vec!["ctrl+x h"]),
        ("app.session.changeDirectory", vec!["ctrl+x w"]),
        ("app.settings.open", vec!["ctrl+x s"]),
        ("app.hotkeys.open", vec!["ctrl+x ?", "ctrl+x k"]),
        ("app.subagent.attach", vec!["ctrl+x n"]),
        ("tui.editor.cursorWordLeft", vec!["ctrl+left"]),
        ("tui.editor.cursorWordRight", vec!["ctrl+right"]),
        ("tui.editor.jumpBackward", vec![]),
        ("tui.editor.deleteWordBackward", vec!["ctrl+w"]),
        ("tui.editor.deleteWordForward", vec!["ctrl+delete"]),
        ("tui.editor.yankPop", vec!["ctrl+x y"]),
        ("tui.editor.redo", vec!["ctrl+x z"]),
        ("tui.editor.undo", vec!["ctrl+-"]),
        ("tui.input.newLine", vec!["shift+enter", "ctrl+x enter"]),
    ];
    let mut wrong = Vec::new();
    for (id, keys) in plan {
        let actual = m.get_keys(id);
        let expected: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        if actual != expected {
            wrong.push(format!("{id}: {actual:?}, the plan says {expected:?}"));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn keeps_the_keys_for_getting_out_and_getting_help_off_alt() {
    let m = manager();
    let must_work_anywhere = [
        "app.interrupt",
        "app.clear",
        "app.exit",
        "app.tools.expand",
        "tui.input.submit",
    ];
    let needing_alt: Vec<&str> = must_work_anywhere
        .into_iter()
        .filter(|id| m.get_keys(id).iter().all(|k| has_alt(k)))
        .collect();
    assert_eq!(needing_alt, Vec::<&str>::new());
}

#[test]
fn binds_nothing_to_an_alt_key_the_parser_cannot_read() {
    let m = manager();
    let re = Regex::new(r"^alt\+[^a-z0-9]$").unwrap();
    let mut unparseable = Vec::new();
    for id in ids() {
        for key in m.get_keys(id) {
            if re.is_match(&key) {
                unparseable.push(format!("{id}: {key}"));
            }
        }
    }
    assert_eq!(unparseable, Vec::<String>::new());
}

#[test]
fn keeps_every_scopes_ids_real() {
    let all = ids();
    for (_, ids) in SCOPES {
        for id in *ids {
            assert!(all.contains(id), "{id}");
        }
    }
}

#[test]
fn leaves_the_emacs_editing_keys_to_the_pickers_query_lines() {
    let m = manager();
    let re = Regex::new(r"^ctrl\+[a-z]$").unwrap();
    let ctrl_letter_verbs: Vec<&str> = SESSION_PICKER_SCOPE
        .iter()
        .chain(MODELS_PICKER_SCOPE)
        .chain(TREE_SCOPE)
        .copied()
        .filter(|id| id.starts_with("app."))
        .filter(|id| m.get_keys(id).iter().any(|k| re.is_match(k)))
        .collect();
    assert_eq!(ctrl_letter_verbs, Vec::<&str>::new());
}

/// The dials: (name, forward, backward).
const DIALS: &[(&str, &str, &str)] = &[
    (
        "agent mode",
        "app.mode.cycleForward",
        "app.mode.cycleBackward",
    ),
    ("model", "app.model.cycleForward", "app.model.cycleBackward"),
    (
        "thinking level",
        "app.thinking.cycleForward",
        "app.thinking.cycleBackward",
    ),
    (
        "tool output",
        "app.view.cycleForward",
        "app.view.cycleBackward",
    ),
    (
        "task ledger",
        "app.tasks.cycleForward",
        "app.tasks.cycleBackward",
    ),
];

#[test]
fn puts_every_dial_on_a_leader_letter_and_its_capital_back() {
    // Agent mode steps with Shift+Tab, and its backward key is the leader's a.
    // The other dials: the leader and a letter forward, the capital back.
    let m = manager();
    let mut wrong = Vec::new();
    for (name, forward, backward) in DIALS {
        let forward_key = m.get_keys(forward).first().cloned().unwrap_or_default();
        let backward_key = m.get_keys(backward).first().cloned().unwrap_or_default();
        if *name == "agent mode" {
            if forward_key != "shift+tab" || backward_key != "ctrl+x a" {
                wrong.push(format!(
                    "{name}: {forward_key} -> {backward_key}, expected shift+tab -> ctrl+x a"
                ));
            }
            continue;
        }
        let Some(letter) = forward_key.strip_prefix("ctrl+x ") else {
            wrong.push(format!(
                "{name}: forward is {forward_key}, expected ctrl+x <letter>"
            ));
            continue;
        };
        let capital = format!("ctrl+x {}", letter.to_uppercase());
        if backward_key != capital {
            wrong.push(format!(
                "{name}: {forward_key} -> {backward_key}, expected {capital}"
            ));
        }
    }
    assert_eq!(wrong, Vec::<String>::new());
}

#[test]
fn gives_each_dial_a_letter_no_other_dial_claims() {
    let m = manager();
    let letters: Vec<String> = DIALS
        .iter()
        .map(|(_, f, _)| m.get_keys(f)[0].clone())
        .collect();
    let set: HashSet<&String> = letters.iter().collect();
    assert_eq!(set.len(), letters.len());
}

#[test]
fn leaves_every_dial_reachable_without_kitty() {
    let m = manager();
    let stranded: Vec<&str> = DIALS
        .iter()
        .filter(|(_, forward, _)| {
            !m.get_keys(forward)
                .iter()
                .any(|k| reachable_without_kitty(k))
        })
        .map(|(name, _, _)| *name)
        .collect();
    assert_eq!(stranded, Vec::<&str>::new());
}

#[test]
fn leaves_no_dial_unbound() {
    let m = manager();
    let unbound: Vec<&str> = DIALS
        .iter()
        .filter(|(_, forward, _)| m.get_keys(forward).is_empty())
        .map(|(name, _, _)| *name)
        .collect();
    assert_eq!(unbound, Vec::<&str>::new());
}

#[test]
fn keeps_the_dials_off_each_others_keys() {
    let ids: Vec<&str> = DIALS.iter().flat_map(|(_, f, b)| [*f, *b]).collect();
    assert_eq!(collisions_within(&ids), Vec::<String>::new());
    assert_eq!(legacy_collisions_within(&ids), Vec::<String>::new());
}

/// The families, in declaration order.
fn families() -> Vec<(&'static str, Regex)> {
    [
        ("Flow", r"^app\.(interrupt|clear|exit|suspend)$"),
        ("Compose", r"^app\.(editor\.external|input\.voiceTranscribe|clipboard\.|message\.)"),
        ("Recall", r"^app\.history\."),
        ("Steer", r"^app\.(mode|model)\.|^app\.thinking\.cycle"),
        ("Read", r"^app\.(view\.|tools\.expand|thinking\.toggle|tasks\.|panel\.focus$)"),
        ("Scroll", r"^app\.scroll\.(pageUp|pageDown|top|bottom|previousMessage|nextMessage|shiftLineUp|shiftLineDown|search)$"),
        ("Go", r"^app\.(session\.(resume|tree|new|fork|changeDirectory)|settings|hotkeys)"),
        (
            "Overlays",
            r"^app\.(options\.|subagent\.(attach|detach)$|panel\.(nextTab|prevTab|up|down|open|cancel|exit)$|scroll\.(lineUp|lineDown|exit|searchInView|searchNext|searchPrevious|focusPrevious|focusNext|expandFocused|copyFocused|focusNewest)|session\.(toggle|rename|delete)|models\.|tree\.)",
        ),
        ("Lists", r"^app\.list\."),
        ("Dialogs", r"^app\.dialog\."),
    ]
    .into_iter()
    .map(|(name, re)| (name, Regex::new(re).unwrap()))
    .collect()
}

#[test]
fn puts_every_app_binding_in_exactly_one_family() {
    let fams = families();
    let mut misfiled = Vec::new();
    for id in ids().into_iter().filter(|id| id.starts_with("app.")) {
        let hits: Vec<&str> = fams
            .iter()
            .filter(|(_, re)| re.is_match(id))
            .map(|(n, _)| *n)
            .collect();
        if hits.len() != 1 {
            misfiled.push(format!(
                "{id}: {}",
                if hits.is_empty() {
                    "no family".to_string()
                } else {
                    hits.join(" + ")
                }
            ));
        }
    }
    assert_eq!(misfiled, Vec::<String>::new());
}

#[test]
fn declares_the_families_in_order_contiguously() {
    let fams = families();
    let mut seen: Vec<&str> = Vec::new();
    for id in ids().into_iter().filter(|id| id.starts_with("app.")) {
        let family = fams
            .iter()
            .find(|(_, re)| re.is_match(id))
            .map_or("?", |(n, _)| *n);
        if seen.last() != Some(&family) {
            seen.push(family);
        }
    }
    assert_eq!(seen, fams.iter().map(|(n, _)| *n).collect::<Vec<_>>());
}

#[test]
fn keeps_every_learned_family_small_enough_to_hold() {
    let m = manager();
    let exempt = ["Overlays", "Scroll"];
    let dial_suffix = Regex::new(r"\.cycle(Forward|Backward)$").unwrap();
    let clipboard = Regex::new(r"^app\.clipboard\..*").unwrap();
    let oversized: Vec<(&str, usize)> = families()
        .into_iter()
        .filter(|(name, _)| !exempt.contains(name))
        .map(|(name, re)| {
            let subjects: HashSet<String> = ids()
                .into_iter()
                .filter(|id| re.is_match(id) && !m.get_keys(id).is_empty())
                .map(|id| {
                    let s = dial_suffix.replace(id, "").to_string();
                    clipboard.replace(&s, "app.clipboard").to_string()
                })
                .collect();
            (name, subjects.len())
        })
        .filter(|(_, size)| *size > 5)
        .collect();
    assert_eq!(oversized, Vec::<(&str, usize)>::new());
}

#[test]
fn prints_a_bare_letter_key_as_it_is_typed_never_capitalised() {
    assert_eq!(format_key_text("n", true), "n");
    assert_eq!(format_key_text("p", true), "p");
    assert_eq!(format_key_text("/", true), "/");
    assert_eq!(format_key_text("ctrl+c", true), "Ctrl+C");
    assert_eq!(format_key_text("pageUp", true), "PageUp");
    if !cfg!(target_os = "macos") {
        assert_eq!(format_key_text("shift+alt+z", true), "Shift+Alt+Z");
    }
}

#[test]
fn gives_every_action_a_description_for_hotkeys() {
    let undocumented: Vec<&str> = keybindings()
        .into_iter()
        .filter(|e| e.description.is_empty())
        .map(|e| e.id)
        .collect();
    assert_eq!(undocumented, Vec::<&str>::new());
}
