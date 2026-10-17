//! Component goldens: the `/` commands popup and the `@` files popup as
//! Mission Control consoles (M7, spec d1/d2) — the heavy frame with the
//! count, `›` on the selection band, the `▸` query row and the hint row.
//! Keys, filtering and events are unchanged; plain path completion keeps
//! the legacy list. Accept a deliberate change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-tui-components autocomplete_console`.

use std::collections::HashMap;

use hoocode_tui_components::{
    AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions, CombinedAutocompleteProvider,
    CommandEntry, ConsoleKind, Editor, SlashCommand,
};
use hoocode_tui_keys::{default_tui_keybindings, KeybindingsManager};
use hoocode_tui_render::assert_golden;
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_util::strip_vt_control_characters;

use crate::editor::ed_with;

fn kb() -> KeybindingsManager {
    KeybindingsManager::new(default_tui_keybindings(), HashMap::new())
}

fn slash(name: &str, description: &str) -> CommandEntry {
    CommandEntry::Slash(SlashCommand {
        name: name.into(),
        description: Some(description.into()),
        argument_hint: None,
        get_argument_completions: None,
    })
}

fn type_text(editor: &mut Editor, kb: &KeybindingsManager, text: &str) {
    for ch in text.chars() {
        editor.handle_input_with(&ch.to_string(), kb);
    }
}

fn provider_editor(
    provider: CombinedAutocompleteProvider,
    width_rows: u16,
) -> (Editor, KeybindingsManager) {
    let mut editor = ed_with(80, width_rows, 0, None);
    editor.set_autocomplete_provider(Box::new(provider));
    (editor, kb())
}

fn slash_provider() -> CombinedAutocompleteProvider {
    CombinedAutocompleteProvider::new(
        vec![
            slash("session", "session info and stats"),
            slash("subagent-stats", "agent reliability"),
            slash("model", "switch the ship"),
        ],
        std::env::temp_dir(),
        None,
    )
}

fn check(name: &str, editor: &mut Editor, width: u16, wants: &[&str]) {
    let rendered = render_golden(&mut *editor, width);
    let text = rendered.split("--- styles ---").next().unwrap_or_default();
    let plain: String = strip_vt_control_characters(text);
    for want in wants {
        assert!(plain.contains(want), "{name}: missing {want} in\n{plain}");
    }
    assert_golden!(name, rendered);
}

/// Typing `/sess` opens the commands console with the filtered count.
#[test]
fn commands_console_counts_the_match() {
    let (mut editor, kb) = provider_editor(slash_provider(), 24);
    type_text(&mut editor, &kb, "/sessio");
    check(
        "commands_console_100",
        &mut editor,
        100,
        &[
            "CONSOLE · commands",
            "1 of 3",
            "› /session",
            "session info and stats",
            "▸ /sess",
            "↑↓ pick · tab complete · enter run",
        ],
    );
}

#[test]
fn commands_console_80() {
    let (mut editor, kb) = provider_editor(slash_provider(), 24);
    type_text(&mut editor, &kb, "/sess");
    check(
        "commands_console_80",
        &mut editor,
        80,
        &["CONSOLE · commands", "2 of 3", "› /session", "▸ /sess"],
    );
}

/// Typing narrows the list; the count follows.
#[test]
fn commands_console_search_counts() {
    let (mut editor, kb) = provider_editor(slash_provider(), 24);
    type_text(&mut editor, &kb, "/sessio");
    let rendered = render_golden(&mut editor, 100);
    let plain: String =
        strip_vt_control_characters(rendered.split("--- styles ---").next().unwrap_or_default());
    assert!(plain.contains("1 of 3"), "missing count in\n{plain}");
}

/// The `@` files console: the file rows with their meta and the files hint.
#[test]
fn files_console_100() {
    let mut editor = ed_with(80, 24, 0, None);
    let kb = kb();
    struct Files;
    impl AutocompleteProvider for Files {
        fn get_suggestions(
            &self,
            _lines: &[String],
            _cursor_line: usize,
            _cursor_col: usize,
            _force: bool,
        ) -> Option<AutocompleteSuggestions> {
            Some(AutocompleteSuggestions {
                items: vec![
                    AutocompleteItem {
                        value: "@src/wrap.rs ".into(),
                        label: "wrap.rs".into(),
                        description: Some("src/wrap.rs".into()),
                    },
                    AutocompleteItem {
                        value: "@tests/wrap_tests.rs ".into(),
                        label: "wrap_tests.rs".into(),
                        description: Some("tests/wrap_tests.rs".into()),
                    },
                ],
                prefix: "@wr".into(),
                console: Some(ConsoleKind::Files),
                total: None,
            })
        }
        fn apply_completion(
            &self,
            lines: &[String],
            cursor_line: usize,
            cursor_col: usize,
            _item: &AutocompleteItem,
            _prefix: &str,
        ) -> hoocode_tui_components::ApplyCompletionResult {
            hoocode_tui_components::ApplyCompletionResult {
                lines: lines.to_vec(),
                cursor_line,
                cursor_col,
            }
        }
    }
    editor.set_autocomplete_provider(Box::new(Files));
    type_text(&mut editor, &kb, "look at @wr");
    // `@` requests are debounced: let the deadline pass, then poll.
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert!(editor.poll_autocomplete());
    check(
        "files_console_100",
        &mut editor,
        100,
        &[
            "CARGO · @ files",
            "2 matches",
            "›",
            "wrap.rs",
            "src/wrap.rs",
            "▸ look at @wr",
            "↑↓ pick · tab load · / into folder",
        ],
    );
}

#[test]
fn files_console_80() {
    let mut editor = ed_with(80, 24, 0, None);
    let kb = kb();
    struct Files;
    impl AutocompleteProvider for Files {
        fn get_suggestions(
            &self,
            _lines: &[String],
            _cursor_line: usize,
            _cursor_col: usize,
            _force: bool,
        ) -> Option<AutocompleteSuggestions> {
            Some(AutocompleteSuggestions {
                items: vec![AutocompleteItem {
                    value: "@src/wrap.rs ".into(),
                    label: "wrap.rs".into(),
                    description: Some("src/wrap.rs".into()),
                }],
                prefix: "@wr".into(),
                console: Some(ConsoleKind::Files),
                total: None,
            })
        }
        fn apply_completion(
            &self,
            lines: &[String],
            cursor_line: usize,
            cursor_col: usize,
            _item: &AutocompleteItem,
            _prefix: &str,
        ) -> hoocode_tui_components::ApplyCompletionResult {
            hoocode_tui_components::ApplyCompletionResult {
                lines: lines.to_vec(),
                cursor_line,
                cursor_col,
            }
        }
    }
    editor.set_autocomplete_provider(Box::new(Files));
    type_text(&mut editor, &kb, "look at @wr");
    // `@` requests are debounced: let the deadline pass, then poll.
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert!(editor.poll_autocomplete());
    check(
        "files_console_80",
        &mut editor,
        80,
        &["CARGO · @ files", "1 match", "wrap.rs", "▸ look at @wr"],
    );
}

/// Plain path completion keeps the legacy list: no console frame.
#[test]
fn path_completion_keeps_the_legacy_list() {
    let (mut editor, kb) = provider_editor(slash_provider(), 24);
    type_text(&mut editor, &kb, "./src/");
    let rendered = render_golden(&mut editor, 100);
    let plain: String =
        strip_vt_control_characters(rendered.split("--- styles ---").next().unwrap_or_default());
    assert!(
        !plain.contains("CONSOLE ·") && !plain.contains("CARGO ·"),
        "path completion drew a console in\n{plain}"
    );
}
