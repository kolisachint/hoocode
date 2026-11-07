//! S17: the composer's pasted-image marker (`[image #1 · clipboard.png]`).
//!
//! The editor holds the marker as one atomic segment and expands it to the
//! temp path on submit; [`PromptRow`] dims the marker on the surface, like
//! the mock's After pane.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::support::lock;
use hoocode_code_tui_app::prompt_row::PromptRow;
use hoocode_tui_components::{
    Editor, EditorHost, EditorOptions, EditorTheme, FrameBorderStyle, SelectListTheme,
};
use hoocode_tui_render::{Component, ComponentHandle};
use hoocode_tui_util::strip_vt_control_characters;

fn theme() -> EditorTheme {
    EditorTheme {
        border_color: Box::new(|t: &str| t.to_string()),
        border_chars: None,
        select_list: Box::new(|| SelectListTheme {
            selected_prefix: Box::new(|t: &str| t.to_string()),
            selected_text: Box::new(|t: &str| t.to_string()),
            description: Box::new(|t: &str| t.to_string()),
            scroll_info: Box::new(|t: &str| t.to_string()),
            no_match: Box::new(|t: &str| t.to_string()),
            cursor: None,
            selected_row: None,
            row: None,
        }),
    }
}

fn composer() -> Editor {
    Editor::new(
        EditorHost {
            rows: Box::new(|| 24),
            request_render: Box::new(|| {}),
        },
        theme(),
        EditorOptions {
            padding_x: Some(0),
            autocomplete_max_visible: None,
            border: Some(FrameBorderStyle::None),
        },
    )
}

fn prompt_row(editor: Editor) -> PromptRow {
    let handle: ComponentHandle = Rc::new(RefCell::new(editor));
    PromptRow::new(
        handle,
        Rc::new(|| 2),
        Rc::new(|_| None),
        Rc::new(|| (Vec::new(), Vec::new())),
        Rc::new(Cell::new(false)),
    )
}

fn stripped(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| strip_vt_control_characters(l))
        .collect()
}

#[test]
fn renders_the_image_marker_dim_on_the_surface() {
    let _g = lock(Some("dark"));
    let mut editor = composer();
    for c in "fix the parser".chars() {
        editor.handle_input(&c.to_string());
    }
    editor.insert_image_at_cursor("/tmp/hoocode-clip-1a2b3c.png", "clipboard.png");

    let mut row = prompt_row(editor);
    let lines = row.render(80);
    let plain = stripped(&lines);

    // The mock's After pane: text, then the dim marker row.
    assert!(plain.iter().any(|l| l.contains("fix the parser")));
    let marker_row = plain
        .iter()
        .find(|l| l.contains("[image #1 · clipboard.png]"))
        .expect("marker row renders");
    assert!(marker_row.contains("›") || marker_row.trim_start().starts_with('['));

    // The marker is dimmed, not plain text.
    let raw = lines
        .iter()
        .find(|l| strip_vt_control_characters(l).contains("[image #1 · clipboard.png]"))
        .expect("raw marker row");
    let at = raw.find("[image #1").expect("marker in raw row");
    assert!(
        raw[..at].ends_with('m'),
        "dim SGR opens right before the marker: {raw:?}"
    );
}

#[test]
fn expanded_text_keeps_the_temp_path_for_the_send_payload() {
    let _g = lock(Some("dark"));
    let mut editor = composer();
    editor.insert_image_at_cursor("/tmp/hoocode-clip-1a2b3c.png", "clipboard.png");
    assert_eq!(editor.get_expanded_text(), "/tmp/hoocode-clip-1a2b3c.png");
}

#[test]
fn dims_paste_markers_on_the_same_row_grammar() {
    let _g = lock(Some("dark"));
    let mut editor = composer();
    editor.set_text("[paste #1 +20 lines]");
    let mut row = prompt_row(editor);
    let lines = row.render(80);
    let raw = &lines[0];
    assert!(strip_vt_control_characters(raw).contains("[paste #1 +20 lines]"));
    let at = raw.find("[paste #1").expect("marker in raw row");
    assert!(
        raw[..at].ends_with('m'),
        "dim SGR opens right before the paste marker: {raw:?}"
    );
}
