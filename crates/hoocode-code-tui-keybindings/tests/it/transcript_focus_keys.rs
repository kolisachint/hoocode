//! U21 P2 transcript focus keys (Muse-only, fast): the pinned-view keys for
//! moving between tool blocks, toggling, copying raw output and pinning the
//! newest, and the dropped expand-all (`ctrl+x o` stays unbound).

use hoocode_code_tui_keybindings::{keybinding, AppKeybindingsManager, Context};
use hoocode_tui_keys::get_keybindings;

fn manager() -> AppKeybindingsManager {
    AppKeybindingsManager::default()
}

#[test]
fn pinned_view_keys_have_their_single_view_defaults() {
    let m = manager();
    assert_eq!(m.get_keys("app.scroll.focusPrevious"), vec!["{"]);
    assert_eq!(m.get_keys("app.scroll.focusNext"), vec!["}"]);
    assert_eq!(m.get_keys("app.scroll.expandFocused"), vec!["enter"]);
    assert_eq!(m.get_keys("app.scroll.copyFocused"), vec!["y"]);
    assert_eq!(m.get_keys("app.scroll.focusNewest"), vec!["ctrl+x u"]);
}

#[test]
fn the_focus_keys_live_in_the_scroll_context() {
    // The runtime reads the scroll view by id (`matches`), where several
    // actions may share a key: `action_for_key` would name the first id in
    // sorted order instead.
    manager().install();
    let live = get_keybindings();
    assert!(live.matches("{", "app.scroll.focusPrevious"));
    assert!(live.matches("}", "app.scroll.focusNext"));
    assert!(live.matches("\r", "app.scroll.expandFocused"));
    assert!(live.matches("y", "app.scroll.copyFocused"));
    assert!(!live.matches("enter", "app.scroll.focusPrevious"));
}

#[test]
fn the_newest_key_is_a_leader_sequence_at_the_prompt() {
    let m = manager();
    let table = m.leader_table(Context::Prompt);
    assert_eq!(
        table.get("ctrl+x u").map(String::as_str),
        Some("app.scroll.focusNewest")
    );
}

#[test]
fn no_focus_binding_takes_ctrl_x_o() {
    // Decision 5 (no expand-all in v1): none of the focus keys answers the
    // old expand-all sequence. The dial's own `ctrl+x o` goes away with it
    // in P5; P2 only adds no binding on the sequence.
    manager().install();
    let live = get_keybindings();
    for id in [
        "app.scroll.focusPrevious",
        "app.scroll.focusNext",
        "app.scroll.expandFocused",
        "app.scroll.copyFocused",
        "app.scroll.focusNewest",
    ] {
        assert!(!live.matches("ctrl+x o", id), "{id} answers ctrl+x o");
    }
}

#[test]
fn escape_clears_the_focus_in_code_not_in_the_keymap() {
    // `esc` clears the focus in the scroll view; it is not a second binding,
    // so the scroll exit keeps the key to itself.
    manager().install();
    let live = get_keybindings();
    assert!(live.matches("\x1b", "app.scroll.exit"));
    assert!(!live.matches("\x1b", "app.scroll.expandFocused"));
    assert!(!live.matches("\x1b", "app.scroll.focusPrevious"));
}

#[test]
fn every_focus_binding_is_declared() {
    for id in [
        "app.scroll.focusPrevious",
        "app.scroll.focusNext",
        "app.scroll.expandFocused",
        "app.scroll.copyFocused",
        "app.scroll.focusNewest",
    ] {
        assert!(keybinding(id).is_some(), "{id} is not declared");
    }
}
