//! U21 P2 transcript focus keys (Muse-only, fast): `{` `}` queue for the
//! mode instead of unpinning, `enter`/`y` queue only while a block is
//! focused, `esc` clears the focus and stays pinned, `End` goes back to live,
//! and the pinned hint row names the focused keys.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use hoocode_ai_types::Content;
use hoocode_code_tui_app::scroll_view::{
    format_scroll_status, install_scroll_view, scroll_slot, FocusIntent, ScrollView,
    TranscriptFocus,
};
use hoocode_code_tui_app::status_line::StatusLineData;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_widgets::tool_chain::ToolBlock;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_tui_components::{Editor, EditorHost, EditorOptions};
use hoocode_tui_render::{Component, ComponentHandle, Container, ScrollStatus, Tui, TuiEvent};
use hoocode_tui_terminal::Terminal;
use serde_json::{json, Value};

use crate::support::lock;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;
const ENTER: &str = "\r";
const ESCAPE: &str = "\x1b";
const END: &str = "\x1b[F";

struct SilentTerminal {
    alternate: Arc<AtomicBool>,
}

impl Terminal for SilentTerminal {
    fn start(&mut self, _: Box<dyn FnMut(&str) + Send>, _: Box<dyn FnMut() + Send>) {}
    fn stop(&mut self) {}
    fn drain_input(&mut self, _: Duration, _: Duration) {}
    fn write(&mut self, _: &str) {}
    fn columns(&self) -> u16 {
        WIDTH
    }
    fn rows(&self) -> u16 {
        HEIGHT
    }
    fn kitty_protocol_active(&self) -> bool {
        false
    }
    fn move_by(&mut self, _: i32) {}
    fn hide_cursor(&mut self) {}
    fn show_cursor(&mut self) {}
    fn clear_line(&mut self) {}
    fn clear_from_cursor(&mut self) {}
    fn clear_screen(&mut self) {}
    fn set_title(&mut self, _: &str) {}
    fn set_progress(&mut self, _: bool) {}
    fn mouse_reporting(&self) -> bool {
        true
    }
    fn set_alternate_screen(&mut self, active: bool) {
        self.alternate.store(active, Ordering::SeqCst);
    }
}

struct Harness {
    ui: Tui,
    editor: Rc<RefCell<Editor>>,
    focus: Rc<RefCell<TranscriptFocus>>,
    view: Rc<RefCell<ScrollView>>,
}

impl Harness {
    fn send(&mut self, data: &str) {
        self.ui.process_event(TuiEvent::input(data));
    }

    fn intents(&mut self) -> Vec<FocusIntent> {
        self.view.borrow_mut().take_focus_intents()
    }
}

/// A settled tool block with one text result, for focus resolution.
fn tool_block(name: &str, id: &str) -> ToolBlock {
    let mut block = ToolExecutionComponent::new(
        name,
        id,
        json!({"path": "src/main.rs"}),
        ToolExecutionOptions::default(),
        None,
        "/work",
    );
    block.update_result(
        ToolResult {
            content: vec![Content::text("line one\nline two\n")],
            details: Value::Null,
            is_error: false,
        },
        false,
    );
    Rc::new(RefCell::new(block))
}

fn setup(busy: bool) -> Harness {
    let _g = lock(Some("dark"));
    AppKeybindingsManager::default().install();
    let alternate = Arc::new(AtomicBool::new(false));
    let mut ui = Tui::new(
        Box::new(SilentTerminal {
            alternate: alternate.clone(),
        }),
        None,
    );
    let transcript = Rc::new(RefCell::new(Container::new()));
    {
        let mut chat = transcript.borrow_mut();
        for i in 1..=120 {
            chat.add_child(block(&format!("line {i}")));
        }
    }
    let editor = Rc::new(RefCell::new(Editor::new(
        EditorHost::default(),
        hoocode_code_tui_theme::get_editor_theme(),
        EditorOptions::default(),
    )));
    ui.add_child(transcript.clone());
    ui.add_child(editor.clone());
    let prompt = editor.clone();
    let view = install_scroll_view(
        &mut ui,
        editor.clone(),
        transcript,
        None,
        Rc::new(|_: &ComponentHandle| false),
        Rc::new(move || prompt.borrow().get_text().is_empty()),
        Rc::new(move || busy),
    );
    let focus = Rc::new(RefCell::new(TranscriptFocus::new()));
    view.borrow_mut().set_focus_state(focus.clone());
    ui.set_focus(Some(editor.clone()));
    let _events = ui.start();
    // Pin the view, as a reader would be.
    ui.process_event(TuiEvent::input("\x1b[5~"));
    assert!(ui.scroll_pinned());
    Harness {
        ui,
        editor,
        focus,
        view,
    }
}

/// Plain text rows, so the transcript is long enough to pin.
struct Block {
    rows: Vec<String>,
}

fn block(label: &str) -> Rc<RefCell<Block>> {
    Rc::new(RefCell::new(Block {
        rows: vec![label.to_string()],
    }))
}

impl Component for Block {
    fn render(&mut self, _width: u16) -> Vec<String> {
        self.rows.clone()
    }
    fn invalidate(&mut self) {}
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

fn text_of(editor: &Rc<RefCell<Editor>>) -> String {
    editor.borrow().get_text()
}

// --- queuing ---------------------------------------------------------------

#[test]
fn open_brace_queues_previous_and_stays_pinned() {
    let mut h = setup(false);
    h.send("{");
    assert!(h.ui.scroll_pinned());
    assert_eq!(text_of(&h.editor), "");
    assert_eq!(h.intents(), vec![FocusIntent::Previous]);
}

#[test]
fn close_brace_queues_next_and_stays_pinned() {
    let mut h = setup(false);
    h.send("}");
    assert!(h.ui.scroll_pinned());
    assert_eq!(h.intents(), vec![FocusIntent::Next]);
}

#[test]
fn braces_over_a_nonempty_prompt_type_as_before() {
    let mut h = setup(false);
    h.editor.borrow_mut().insert_text_at_cursor("gi");
    h.send("{");
    assert!(!h.ui.scroll_pinned());
    assert_eq!(text_of(&h.editor), "gi{");
    assert!(h.intents().is_empty());
}

#[test]
fn enter_without_a_focus_leaves_the_view_as_before() {
    let mut h = setup(false);
    h.send(ENTER);
    assert!(!h.ui.scroll_pinned());
    assert!(h.intents().is_empty());
}

#[test]
fn enter_with_a_focus_queues_toggle_and_stays_pinned() {
    let mut h = setup(false);
    h.focus.borrow_mut().focus("call-1");
    h.send(ENTER);
    assert!(h.ui.scroll_pinned());
    assert_eq!(h.intents(), vec![FocusIntent::Toggle]);
}

#[test]
fn y_without_a_focus_types_as_before() {
    let mut h = setup(false);
    h.send("y");
    assert!(!h.ui.scroll_pinned());
    assert_eq!(text_of(&h.editor), "y");
    assert!(h.intents().is_empty());
}

#[test]
fn y_with_a_focus_queues_copy_and_stays_pinned() {
    let mut h = setup(false);
    h.focus.borrow_mut().focus("call-1");
    h.send("y");
    assert!(h.ui.scroll_pinned());
    assert_eq!(text_of(&h.editor), "");
    assert_eq!(h.intents(), vec![FocusIntent::Copy]);
}

#[test]
fn focusing_keys_do_not_fire_inside_an_open_search() {
    let mut h = setup(false);
    h.send("/");
    assert!(h.ui.scroll_search_active());
    h.send("{");
    assert!(h.intents().is_empty());
    assert_eq!(h.ui.scroll_search_query(), "{");
}

// --- esc and End --------------------------------------------------------------

#[test]
fn escape_with_a_focus_clears_it_and_stays_pinned() {
    let mut h = setup(false);
    h.focus.borrow_mut().focus("call-1");
    h.send(ESCAPE);
    assert!(h.ui.scroll_pinned());
    assert!(!h.focus.borrow().is_focused());
    assert!(h.intents().is_empty());
}

#[test]
fn escape_without_a_focus_leaves_the_view_as_before() {
    let mut h = setup(false);
    h.send(ESCAPE);
    assert!(!h.ui.scroll_pinned());
}

#[test]
fn end_clears_the_focus_and_goes_back_to_live() {
    let mut h = setup(false);
    h.focus.borrow_mut().focus("call-9");
    h.send(END);
    assert!(!h.ui.scroll_pinned());
    assert!(!h.focus.borrow().is_focused());
}

#[test]
fn end_goes_back_to_live_while_busy() {
    let mut h = setup(true);
    h.focus.borrow_mut().focus("call-9");
    h.send(END);
    assert!(!h.ui.scroll_pinned());
    assert!(!h.focus.borrow().is_focused());
}

// --- resolution ---------------------------------------------------------------

#[test]
fn resolve_finds_the_focused_block_in_order() {
    let _g = lock(Some("dark"));
    let blocks = vec![tool_block("Read", "call-1"), tool_block("Shell", "call-2")];
    let focus = Rc::new(RefCell::new(TranscriptFocus::new()));
    assert!(focus.borrow().resolve(&blocks).is_none());
    focus.borrow_mut().focus("call-2");
    let found = focus.borrow().resolve(&blocks).expect("focused");
    assert_eq!(found.borrow().tool_call_id(), "call-2");
}

#[test]
fn resolve_misses_a_call_a_retry_took_out() {
    let _g = lock(Some("dark"));
    let mut focus = TranscriptFocus::new();
    focus.focus("call-1");
    assert!(focus.resolve(&[]).is_none());
    let blocks = vec![tool_block("Read", "call-2")];
    assert!(focus.resolve(&blocks).is_none());
}

// --- hints --------------------------------------------------------------------

fn status() -> ScrollStatus {
    hoocode_tui_render::ScrollStatus {
        top: 43,
        bottom: 61,
        total: 78,
        view_height: 19,
        at_top: false,
        at_bottom: false,
        width: 100,
        search: None,
        picker: false,
        new_lines: 3,
    }
}

fn base() -> StatusLineData {
    StatusLineData {
        session: Some("amber-harbor".into()),
        model: Some("claude-haiku-5-5".into()),
        ..Default::default()
    }
}

#[test]
fn the_focused_slot_names_enter_y_and_esc() {
    let _g = lock(Some("dark"));
    AppKeybindingsManager::default().install();
    let (scroll, search) = scroll_slot(&status(), true);
    assert!(search.is_none());
    let keys: Vec<String> = scroll
        .expect("scroll")
        .keys
        .iter()
        .map(|key| format!("{} {}", key.key, key.action))
        .collect();
    assert_eq!(keys, vec!["enter open", "y copy", "esc clear"]);
}

#[test]
fn the_unfocused_slot_keeps_esc_live() {
    let _g = lock(Some("dark"));
    AppKeybindingsManager::default().install();
    let (scroll, _) = scroll_slot(&status(), false);
    let keys: Vec<String> = scroll
        .expect("scroll")
        .keys
        .iter()
        .map(|key| format!("{} {}", key.key, key.action))
        .collect();
    assert_eq!(keys, vec!["esc live"]);
}

#[test]
fn the_focused_status_row_reads_the_focus_keys() {
    let _g = lock(Some("dark"));
    AppKeybindingsManager::default().install();
    let row = crate::support::plain(&format_scroll_status(&status(), &base(), true));
    assert!(row.contains("enter open"), "{row}");
    assert!(row.contains("y copy"), "{row}");
    assert!(row.contains("esc clear"), "{row}");
}
