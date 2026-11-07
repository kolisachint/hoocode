//! U21 P2 `/perf` check: the transcript focus path at scale. Two hundred
//! settled tool blocks render and two thousand focus keys queue with no loop
//! stall, and the `/perf` report says so. The frame build stays far under
//! the stall threshold, so the focus keys cannot jank the pinned view.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use hoocode_ai_types::Content;
use hoocode_code_tui_app::perf::{format_report, ProcSample, Recorder, Snapshot, STALL_THRESHOLD};
use hoocode_code_tui_app::scroll_view::{install_scroll_view, TranscriptFocus};
use hoocode_code_tui_keybindings::AppKeybindingsManager;
use hoocode_code_tui_widgets::tool_chain::ToolBlock;
use hoocode_code_tui_widgets::tool_execution::{ToolExecutionComponent, ToolExecutionOptions};
use hoocode_code_tui_widgets::tool_signal::ToolResult;
use hoocode_tui_components::{Editor, EditorHost, EditorOptions};
use hoocode_tui_render::{ComponentHandle, Container, Tui, TuiEvent};
use hoocode_tui_terminal::Terminal;
use serde_json::{json, Value};

use crate::support::lock;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;
/// Tool blocks in the scaled transcript.
const BLOCKS: usize = 200;
/// Focus keys through the pinned view.
const KEYS: usize = 2000;

struct SilentTerminal;

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
    fn set_alternate_screen(&mut self, _: bool) {}
}

/// A settled Read block with a few text lines, as the transcript holds it.
fn tool_block(id: usize) -> ToolBlock {
    let mut block = ToolExecutionComponent::new(
        "Read",
        &format!("call-{id}"),
        json!({"path": "src/main.rs"}),
        ToolExecutionOptions::default(),
        None,
        "/work",
    );
    block.update_result(
        ToolResult {
            content: vec![Content::text("line one\nline two\nline three\n")],
            details: Value::Null,
            is_error: false,
        },
        false,
    );
    Rc::new(RefCell::new(block))
}

/// A pinned transcript of `BLOCKS` settled blocks, with the focus state set.
fn harness() -> (Tui, Rc<RefCell<TranscriptFocus>>) {
    let _g = lock(Some("dark"));
    AppKeybindingsManager::default().install();
    let mut ui = Tui::new(Box::new(SilentTerminal), None);
    let chat = Rc::new(RefCell::new(Container::new()));
    for id in 0..BLOCKS {
        chat.borrow_mut().add_child(tool_block(id));
    }
    let editor = Rc::new(RefCell::new(Editor::new(
        EditorHost::default(),
        hoocode_code_tui_theme::get_editor_theme(),
        EditorOptions::default(),
    )));
    ui.add_child(chat.clone());
    ui.add_child(editor.clone());
    let prompt = editor.clone();
    let view = install_scroll_view(
        &mut ui,
        editor.clone(),
        chat,
        None,
        Rc::new(|_: &ComponentHandle| false),
        Rc::new(move || prompt.borrow().get_text().is_empty()),
        Rc::new(|| false),
    );
    let focus = Rc::new(RefCell::new(TranscriptFocus::new()));
    view.borrow_mut().set_focus_state(focus.clone());
    ui.set_focus(Some(editor));
    let _events = ui.start();
    ui.process_event(TuiEvent::input("\x1b[5~"));
    assert!(ui.scroll_pinned());
    (ui, focus)
}

#[test]
fn two_thousand_focus_keys_queue_with_no_loop_stall() {
    let (mut ui, _focus) = harness();
    let mut recorder = Recorder::new();
    // Alternate the move keys, as stepping back and forth would.
    for i in 0..KEYS {
        let key = if i % 2 == 0 { "}" } else { "{" };
        let started = Instant::now();
        ui.process_event(TuiEvent::input(key));
        recorder.record_iteration(started.elapsed());
        assert!(ui.scroll_pinned(), "key {i} unpinned the view");
    }
    let stats = recorder.stats();
    assert_eq!(stats.stalls, 0, "a focus key stalled the loop");
    let report = format_report(&Snapshot {
        proc: ProcSample::default(),
        stats,
    });
    assert!(report.contains("stalls (loop > 500 ms): 0"), "{report}");
}

#[test]
fn the_scaled_transcript_renders_far_under_the_stall_threshold() {
    let (mut ui, _focus) = harness();
    let mut recorder = Recorder::new();
    for _ in 0..30 {
        let started = Instant::now();
        let rows = ui.render(WIDTH);
        let build = started.elapsed();
        assert!(!rows.is_empty());
        recorder.record_frame(hoocode_tui_render::FrameTiming {
            build,
            write: Duration::ZERO,
        });
    }
    let stats = recorder.stats();
    assert_eq!(stats.frames, 30);
    let build = stats.frame_build.expect("frame samples");
    assert!(
        build.p99 < STALL_THRESHOLD,
        "p99 frame build {}ms hits the {}ms stall line",
        build.p99.as_secs_f64() * 1000.0,
        STALL_THRESHOLD.as_millis()
    );
}

#[test]
fn focus_resolution_scans_two_hundred_blocks_fast() {
    let _g = lock(Some("dark"));
    let blocks: Vec<ToolBlock> = (0..BLOCKS).map(tool_block).collect();
    assert!(TranscriptFocus::new().resolve(&blocks).is_none());
    let started = Instant::now();
    for id in 0..1000 {
        let mut focus = TranscriptFocus::new();
        focus.focus(&format!("call-{}", id % BLOCKS));
        assert!(focus.resolve(&blocks).is_some());
    }
    assert!(
        started.elapsed() < STALL_THRESHOLD,
        "a thousand resolves must not take a stall"
    );
    // A miss (a retry took the chain out) resolves to nothing.
    let mut gone = TranscriptFocus::new();
    gone.focus("call-999");
    assert!(gone.resolve(&blocks).is_none());
}
