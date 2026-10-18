//! Component goldens: the `/` commands popup and the `@` files popup as a
//! frameless drop-down under the line — one surface at a time, so the comms
//! frame stays the single box. No heavy frame, no count title, no `▸` query
//! row repeating the line, no hint row. Keys, filtering and events are
//! unchanged; plain path completion keeps the same list. Accept a deliberate
//! change with
//! `UPDATE_GOLDENS=1 cargo nextest run -p hoocode-tui-components autocomplete_dropdown`.

use std::collections::HashMap;

use hoocode_tui_components::{
    AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions, CombinedAutocompleteProvider,
    CommandEntry, Editor, SlashCommand,
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
    // The app prefixes the line with `▸`; mirror it so the goldens show it.
    editor.prompt_prefix = "▸".into();
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

fn plain(rendered: &str) -> String {
    strip_vt_control_characters(rendered.split("--- styles ---").next().unwrap_or_default())
}

/// The drop-down draws no box of its own: no heavy corners, no console
/// title, and the line's `▸` stays the only one on screen.
fn assert_frameless(name: &str, rendered: &str, text: &str) {
    for marker in ["┏", "┓", "┗", "┛", "CONSOLE ·", "CARGO ·", "↑↓ pick"] {
        assert!(
            !text.contains(marker),
            "{name}: drop-down draws a frame ({marker}) in\n{text}\n{rendered}"
        );
    }
    assert_eq!(
        text.matches('▸').count(),
        1,
        "{name}: the query row repeats the line's ▸ in\n{text}\n{rendered}"
    );
}

fn check(name: &str, editor: &mut Editor, width: u16, wants: &[&str]) {
    let rendered = render_golden(&mut *editor, width);
    let text = plain(&rendered);
    for want in wants {
        assert!(text.contains(want), "{name}: missing {want} in\n{text}");
    }
    assert_frameless(name, &rendered, &text);
    assert_golden!(name, rendered);
}

/// Typing `/sess` opens the commands drop-down with the filtered rows.
#[test]
fn slash_dropdown_lists_the_match() {
    let (mut editor, kb) = provider_editor(slash_provider(), 24);
    type_text(&mut editor, &kb, "/sessio");
    check(
        "slash_dropdown_100",
        &mut editor,
        100,
        &["› session", "session info and stats"],
    );
}

#[test]
fn slash_dropdown_80() {
    let (mut editor, kb) = provider_editor(slash_provider(), 24);
    type_text(&mut editor, &kb, "/sess");
    check(
        "slash_dropdown_80",
        &mut editor,
        80,
        &["› session", "subagent-stats"],
    );
}

/// Typing narrows the list; non-matches leave it.
#[test]
fn slash_dropdown_search_filters() {
    let (mut editor, kb) = provider_editor(slash_provider(), 24);
    type_text(&mut editor, &kb, "/sessio");
    let rendered = render_golden(&mut editor, 100);
    let text = plain(&rendered);
    assert!(text.contains("session"), "missing the match in\n{text}");
    assert!(
        !text.contains("subagent-stats"),
        "unfiltered row stays in\n{text}"
    );
    assert_frameless("slash_dropdown_search_filters", &rendered, &text);
}

struct Files {
    items: Vec<AutocompleteItem>,
}

impl AutocompleteProvider for Files {
    fn get_suggestions(
        &self,
        _lines: &[String],
        _cursor_line: usize,
        _cursor_col: usize,
        _force: bool,
    ) -> Option<AutocompleteSuggestions> {
        Some(AutocompleteSuggestions {
            items: self.items.clone(),
            prefix: "@wr".into(),
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

fn files_editor(items: Vec<AutocompleteItem>) -> (Editor, KeybindingsManager) {
    let mut editor = ed_with(80, 24, 0, None);
    editor.prompt_prefix = "▸".into();
    let kb = kb();
    editor.set_autocomplete_provider(Box::new(Files { items }));
    (editor, kb)
}

fn file(label: &str, description: &str) -> AutocompleteItem {
    AutocompleteItem {
        value: format!("@{label} "),
        label: label.into(),
        description: Some(description.into()),
    }
}

/// The `@` files drop-down: the file rows with their meta, no frame.
#[test]
fn files_dropdown_100() {
    let (mut editor, kb) = files_editor(vec![
        file("wrap.rs", "src/wrap.rs"),
        file("wrap_tests.rs", "tests/wrap_tests.rs"),
    ]);
    type_text(&mut editor, &kb, "look at @wr");
    // `@` requests are debounced: let the deadline pass, then poll.
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert!(editor.poll_autocomplete());
    check(
        "files_dropdown_100",
        &mut editor,
        100,
        &["›", "wrap.rs", "src/wrap.rs", "wrap_tests.rs"],
    );
}

#[test]
fn files_dropdown_80() {
    let (mut editor, kb) = files_editor(vec![file("wrap.rs", "src/wrap.rs")]);
    type_text(&mut editor, &kb, "look at @wr");
    // `@` requests are debounced: let the deadline pass, then poll.
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert!(editor.poll_autocomplete());
    check(
        "files_dropdown_80",
        &mut editor,
        80,
        &["wrap.rs", "src/wrap.rs"],
    );
}

/// Plain path completion keeps the same frameless list.
#[test]
fn path_completion_keeps_the_legacy_list() {
    let (mut editor, kb) = provider_editor(slash_provider(), 24);
    type_text(&mut editor, &kb, "./src/");
    let rendered = render_golden(&mut editor, 100);
    let text = plain(&rendered);
    assert_frameless("path_completion_keeps_the_legacy_list", &rendered, &text);
}
