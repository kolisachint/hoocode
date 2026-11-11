//! Component goldens: the `/` commands menu and the `@` files menu as the
//! prompt row draws them: pmenu style, a pane-colour block above the prompt,
//! as wide as its longest row, its text under the typed token, the selected
//! row marked ` › `. Dark, light and monochrome (light borders, no colour).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use crate::support::{lock, look, Look};
use hoocode_code_tui_app::prompt_row::PromptRow;
use hoocode_code_tui_theme::get_editor_theme;
use hoocode_tui_components::{
    ApplyCompletionResult, AutocompleteItem, AutocompleteProvider, AutocompleteSuggestions,
    CombinedAutocompleteProvider, CommandEntry, Editor, EditorHost, EditorOptions,
    FrameBorderStyle, SlashCommand,
};
use hoocode_tui_render::golden::render_golden;
use hoocode_tui_render::{assert_golden, Component, ComponentHandle};
use hoocode_tui_util::strip_vt_control_characters;

/// A prompt row over a live editor, the editor's menu read back through
/// [`Editor::render_completion`] as the app does.
fn prompt(provider: Box<dyn AutocompleteProvider>) -> (Rc<RefCell<Editor>>, PromptRow) {
    let editor = Rc::new(RefCell::new(Editor::new(
        EditorHost {
            rows: Box::new(|| 24),
            request_render: Box::new(|| {}),
        },
        get_editor_theme(),
        EditorOptions {
            border: Some(FrameBorderStyle::None),
            ..EditorOptions::default()
        },
    )));
    {
        let mut e = editor.borrow_mut();
        e.set_autocomplete_provider(provider);
        e.set_inline_completion(false);
    }
    let menu = editor.clone();
    let handle: ComponentHandle = editor.clone();
    let row = PromptRow::new(
        handle,
        Rc::new(|| 1),
        Rc::new(move |max| {
            menu.try_borrow_mut()
                .ok()
                .and_then(|mut e| e.render_completion(max))
        }),
        Rc::new(|| (Vec::new(), Vec::new())),
        Rc::new(Cell::new(false)),
    );
    (editor, row)
}

fn type_text(editor: &Rc<RefCell<Editor>>, text: &str) {
    for ch in text.chars() {
        editor.borrow_mut().handle_input(&ch.to_string());
    }
}

fn slash_provider() -> CombinedAutocompleteProvider {
    let command = |name: &str, description: &str| {
        CommandEntry::Slash(SlashCommand {
            name: name.into(),
            description: Some(description.into()),
            argument_hint: None,
            get_argument_completions: None,
        })
    };
    CombinedAutocompleteProvider::new(
        vec![
            command("model", "switch the model"),
            command("resume", "resume a session from history"),
            command("settings", "open settings"),
            command("hotkeys", "show the keys"),
        ],
        std::env::temp_dir(),
        None,
    )
}

fn slash_menu(name: &str, variant: Look) {
    let _g = lock(Some("dark"));
    look(variant);
    let (editor, mut row) = prompt(Box::new(slash_provider()));
    type_text(&editor, "/");
    let rendered = render_golden(&mut row, 100);
    let text =
        strip_vt_control_characters(rendered.split("--- styles ---").next().unwrap_or_default());
    assert!(text.contains("› model"), "{name}: selected row in\n{text}");
    assert!(
        text.contains("switch the model"),
        "{name}: description in\n{text}"
    );
    assert_golden!(name, rendered);
}

#[test]
fn slash_menu_dark_100() {
    slash_menu("dialog_slash_menu_dark_100", Look::Dark);
}

#[test]
fn slash_menu_light_100() {
    slash_menu("dialog_slash_menu_light_100", Look::Light);
}

#[test]
fn slash_menu_mono_100() {
    slash_menu("dialog_slash_menu_mono_100", Look::Mono);
}

/// The `@` files provider with fixed rows, so the golden does not read the disk.
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
    ) -> ApplyCompletionResult {
        ApplyCompletionResult {
            lines: lines.to_vec(),
            cursor_line,
            cursor_col,
        }
    }
}

fn file(label: &str, description: &str) -> AutocompleteItem {
    AutocompleteItem {
        value: format!("@{label} "),
        label: label.into(),
        description: Some(description.into()),
    }
}

fn file_menu(name: &str, variant: Look) {
    let _g = lock(Some("dark"));
    look(variant);
    let (editor, mut row) = prompt(Box::new(Files {
        items: vec![
            file("wrap.rs", "src/wrap.rs"),
            file("wrap_tests.rs", "tests/wrap_tests.rs"),
        ],
    }));
    type_text(&editor, "look at @wr");
    // `@` requests are debounced: let the deadline pass, then poll.
    thread::sleep(Duration::from_millis(30));
    assert!(editor.borrow_mut().poll_autocomplete());
    let rendered = render_golden(&mut row, 100);
    let text =
        strip_vt_control_characters(rendered.split("--- styles ---").next().unwrap_or_default());
    assert!(
        text.contains("› wrap.rs"),
        "{name}: selected row in\n{text}"
    );
    assert!(
        text.contains("src/wrap.rs"),
        "{name}: description in\n{text}"
    );
    assert_golden!(name, rendered);
}

#[test]
fn file_menu_dark_100() {
    file_menu("dialog_file_menu_dark_100", Look::Dark);
}

#[test]
fn file_menu_light_100() {
    file_menu("dialog_file_menu_light_100", Look::Light);
}

#[test]
fn file_menu_mono_100() {
    file_menu("dialog_file_menu_mono_100", Look::Mono);
}
