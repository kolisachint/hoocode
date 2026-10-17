//! The full-screen scroll view (M1d): the transcript is the body of the screen,
//! and the pinned view keeps the keys, the `N new` count, the scrollbar and the
//! jump to a user message through the body.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hoocode_code_tui_app::scroll_view::install_scroll_view;
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_theme::{get_editor_theme, init_theme};
use hoocode_tui_components::{Editor, EditorHost, EditorOptions};
use hoocode_tui_render::{Component, ComponentHandle, Container, Tui, TuiEvent};
use hoocode_tui_terminal::Terminal;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;

/// Records every byte the TUI writes, and reports mouse reports as on.
#[derive(Clone, Default)]
struct Output(Arc<Mutex<String>>);

impl Output {
    fn text(&self) -> String {
        self.0.lock().unwrap().clone()
    }
    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

struct RecordingTerminal {
    output: Output,
    alternate: Arc<AtomicBool>,
}

impl Terminal for RecordingTerminal {
    fn start(&mut self, _: Box<dyn FnMut(&str) + Send>, _: Box<dyn FnMut() + Send>) {}
    fn stop(&mut self) {}
    fn drain_input(&mut self, _: Duration, _: Duration) {}
    fn write(&mut self, data: &str) {
        self.output.0.lock().unwrap().push_str(data);
    }
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

/// A transcript block: `rows` rows, labelled, and a user message when `user`.
struct Block {
    rows: Vec<String>,
    user: bool,
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

fn block(label: &str, rows: usize, user: bool) -> ComponentHandle {
    Rc::new(RefCell::new(Block {
        rows: (0..rows).map(|i| format!("{label}:{i}")).collect(),
        user,
    }))
}

fn is_user(child: &ComponentHandle) -> bool {
    child
        .borrow()
        .as_any()
        .and_then(|a| a.downcast_ref::<Block>())
        .is_some_and(|b| b.user)
}

struct Screen {
    ui: Tui,
    output: Output,
    chat: Rc<RefCell<Container>>,
    alternate: Arc<AtomicBool>,
}

impl Screen {
    fn send(&mut self, data: &str) {
        self.ui.process_event(TuiEvent::input(data));
    }

    fn top(&self) -> i64 {
        self.ui.get_scroll_position().expect("pinned").0
    }

    fn view(&self) -> i64 {
        self.ui.get_scroll_position().expect("pinned").2
    }
}

/// The app's full-screen tree, with fixed content: a HUD row, a gap, the body
/// (a header and the chat), and the prompt. The body is 1 (header) + 10 + 1 +
/// 30 + 1 + 60 rows: the user messages start at body rows 11 and 42.
fn screen() -> Screen {
    init_theme(Some("dark"), false);
    AppKeybindingsManager::default().install();
    let output = Output::default();
    let alternate = Arc::new(AtomicBool::new(false));
    let mut ui = Tui::new(
        Box::new(RecordingTerminal {
            output: output.clone(),
            alternate: alternate.clone(),
        }),
        None,
    );
    ui.set_fullscreen(true);
    ui.add_child(block("HUD", 1, false));
    ui.add_child(block("gap", 1, false));

    let chat = Rc::new(RefCell::new(Container::new()));
    {
        let mut chat = chat.borrow_mut();
        chat.add_child(block("a", 10, false));
        chat.add_child(block("u1", 1, true));
        chat.add_child(block("b", 30, false));
        chat.add_child(block("u2", 1, true));
        chat.add_child(block("c", 60, false));
    }
    let body = Rc::new(RefCell::new(Container::new()));
    body.borrow_mut().add_child(block("header", 1, false));
    let chat_handle: ComponentHandle = chat.clone();
    body.borrow_mut().add_child(chat_handle);
    let body_handle: ComponentHandle = body.clone();
    ui.add_child(body_handle.clone());
    ui.set_fullscreen_body(Some(body_handle));

    let editor = Rc::new(RefCell::new(Editor::new(
        EditorHost::default(),
        get_editor_theme(),
        EditorOptions::default(),
    )));
    let editor_handle: ComponentHandle = editor.clone();
    ui.add_child(editor_handle.clone());
    install_scroll_view(
        &mut ui,
        editor_handle.clone(),
        chat.clone(),
        Some(body.clone()),
        Rc::new(is_user),
    );
    ui.set_focus(Some(editor_handle));
    let _events = ui.start();
    Screen {
        ui,
        output,
        chat,
        alternate,
    }
}

const SHIFT_UP: &str = "\x1b[1;2A";
const SHIFT_DOWN: &str = "\x1b[1;2B";
const CTRL_UP: &str = "\x1bOa";
const CTRL_DOWN: &str = "\x1bOb";
const CTRL_END: &str = "\x1b[8^";
const PAGE_UP: &str = "\x1b[5~";
const WHEEL_UP: &str = "\x1b[<64;1;1M";
/// A left press on the last column, screen row 3 (the first body row).
const PRESS_SCROLLBAR_TOP: &str = "\x1b[<0;100;3M";

#[test]
fn shift_arrows_move_one_line_and_pin_from_the_tail() {
    let mut s = screen();
    assert!(s.ui.get_scroll_position().is_none(), "live to start");
    s.send(SHIFT_UP);
    let pinned = s.top();
    assert!(s.ui.scroll_pinned(), "shift+up pins from the tail");
    s.send(SHIFT_UP);
    assert_eq!(s.top(), pinned - 1, "one line up");
    s.send(SHIFT_DOWN);
    assert_eq!(s.top(), pinned, "one line down");
}

#[test]
fn shift_down_at_the_tail_is_consumed_and_does_nothing() {
    let mut s = screen();
    s.send(SHIFT_DOWN);
    assert!(s.ui.get_scroll_position().is_none(), "still live");
}

#[test]
fn jump_to_user_message_finds_the_chat_inside_the_body() {
    let mut s = screen();
    // The last user message is at body row 42; the view lands three rows above it.
    s.send(CTRL_UP);
    assert_eq!(s.top(), 39, "previous message, from the tail");
    // The one before is at body row 11.
    s.send(CTRL_UP);
    assert_eq!(s.top(), 8, "previous again");
    s.send(CTRL_DOWN);
    assert_eq!(s.top(), 8, "next user message, already in view");
}

#[test]
fn ctrl_end_returns_to_live_and_page_up_pins() {
    let mut s = screen();
    s.send(PAGE_UP);
    assert!(s.ui.scroll_pinned(), "page up pins");
    s.send(CTRL_END);
    assert!(s.ui.get_scroll_position().is_none(), "ctrl+end is live");
}

#[test]
fn the_bar_counts_lines_that_arrived_while_pinned() {
    let mut s = screen();
    s.send(SHIFT_UP);
    s.output.clear();
    s.chat.borrow_mut().add_child(block("n", 5, false));
    s.ui.request_render(true);
    let out = s.output.text();
    assert!(out.contains("\u{2193} 5 new"), "the bar shows the count");
    assert!(s.alternate.load(Ordering::SeqCst), "still full-screen");
}

#[test]
fn a_press_on_the_scrollbar_above_the_thumb_pages_up() {
    let mut s = screen();
    s.send(SHIFT_UP);
    // Pin low, then page to the bottom area so the thumb sits low.
    s.ui.scroll_to_row(i64::MAX, None);
    let before = s.top();
    let view = s.view();
    s.send(PRESS_SCROLLBAR_TOP);
    assert_eq!(s.top(), (before - (view - 2)).max(0), "one page up");
}

#[test]
fn the_wheel_pins_and_moves_three_rows() {
    let mut s = screen();
    s.send(WHEEL_UP);
    assert!(s.ui.scroll_pinned(), "wheel up pins");
    let top = s.top();
    s.send(WHEEL_UP);
    assert_eq!(s.top(), top - 3, "three rows per notch");
}

#[test]
fn the_screen_starts_on_the_alternate_screen_with_the_hud_on_row_one() {
    let s = screen();
    assert!(
        s.alternate.load(Ordering::SeqCst),
        "the alternate screen is on"
    );
    assert!(
        s.output.text().contains("\x1b[1;1H\x1b[2KHUD:0"),
        "the HUD row is the first row"
    );
}
