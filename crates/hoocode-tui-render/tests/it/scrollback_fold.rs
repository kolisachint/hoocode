//! The terminal's scrollback when a block above the bottom folds.
//!
//! The user saw two stacked task panels: a stale stub (top border and first
//! row, `2/5`) in the scrollback over the live one (`0/5`). A prompt that
//! grew pushed the panel's top rows into the scrollback; at submit the prompt
//! folded and the panel changed, the buffer shrank, and the window moved back
//! over rows already in the scrollback. Painting that window in place left the
//! old copies of those rows above it. Now the window is held on a shrink (the
//! rows below go blank), and a change to a row in the scrollback is a cleared
//! full redraw; a small shrink costs no full redraw at all.

use crate::support::*;
use hoocode_tui_render::{FlexSpacer, Tui};
use std::cell::RefCell;
use std::rc::Rc;

const WIDTH: u16 = 30;
const HEIGHT: u16 = 12;
const H: usize = HEIGHT as usize;

struct Setup {
    tui: Tui,
    term: Handle,
    chat: Rc<RefCell<Lines>>,
    panel: Rc<RefCell<Lines>>,
    prompt: Rc<RefCell<Lines>>,
}

/// Fill, a long chat, a three-row panel, a one-row prompt.
fn setup() -> Setup {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    let fill = Rc::new(RefCell::new(FlexSpacer::new()));
    let chat = Lines::body(H * 2);
    let panel = Lines::new(&["panel top 2/5", "panel row a", "panel foot"]);
    let prompt = Lines::new(&["prompt"]);
    tui.add_child(fill.clone());
    tui.add_child(chat.clone());
    tui.add_child(panel.clone());
    tui.add_child(prompt.clone());
    tui.set_flex_spacer(Some(fill));
    let _events = tui.start();
    Setup {
        tui,
        term,
        chat,
        panel,
        prompt,
    }
}

impl Setup {
    fn prompt_rows(&mut self, rows: usize) {
        let mut lines = vec!["prompt".to_string()];
        lines.extend((1..rows).map(|i| format!("prompt line {i}")));
        self.prompt.borrow_mut().0 = lines;
        self.tui.request_render(false);
    }

    fn count(&self, row: &str) -> usize {
        self.term
            .scroll_buffer()
            .iter()
            .filter(|l| l.as_str() == row)
            .count()
    }
}

#[test]
fn a_panel_pushed_into_the_scrollback_leaves_no_stale_copy_when_the_prompt_folds() {
    let mut s = setup();
    // The prompt grows past the panel: its rows go into the scrollback.
    s.prompt_rows(H);
    assert!(!s.term.screen().iter().any(|l| l.starts_with("panel")));
    // Submit: the prompt folds, the panel changes and shrinks, the chat gains
    // a row. The buffer is shorter, so the window moves back.
    *s.panel.borrow_mut() = Lines(vec!["panel top 0/5".into(), "panel foot".into()]);
    s.chat.borrow_mut().set_count(H * 2 + 1);
    s.prompt_rows(1);
    let screen = s.term.screen();
    assert_eq!(screen[H - 1], "prompt", "{screen:?}");
    assert_eq!(screen[H - 3], "panel top 0/5", "{screen:?}");
    let all = s.term.scroll_buffer();
    assert_eq!(s.count("panel top 2/5"), 0, "stale panel left: {all:#?}");
    assert_eq!(s.count("panel row a"), 0, "stale panel left: {all:#?}");
    assert_eq!(s.count("panel top 0/5"), 1, "{all:#?}");
    assert_eq!(s.count("panel foot"), 1, "{all:#?}");
}

#[test]
fn a_folding_picker_leaves_each_chat_row_once_in_the_scrollback() {
    let mut s = setup();
    s.prompt_rows(6);
    s.prompt_rows(1);
    let all = s.term.scroll_buffer();
    for i in 1..=H * 2 {
        assert_eq!(s.count(&format!("body {i}")), 1, "body {i}: {all:#?}");
    }
    // The buffer is the scrollback plus the screen, row for row. The window
    // was held, so the five rows the prompt gave up are blank below it.
    let mut all = all;
    assert!(all.split_off(all.len() - 5).iter().all(String::is_empty));
    let mut expected: Vec<String> = (1..=H * 2).map(|i| format!("body {i}")).collect();
    expected.extend(["panel top 2/5", "panel row a", "panel foot", "prompt"].map(String::from));
    assert_eq!(all, expected);
}

/// An autocomplete-like block: grown by five rows, then narrowed one row per
/// keystroke. No step is a full redraw, the window holds, and the rows above
/// are never drawn again.
#[test]
fn narrowing_a_list_one_row_at_a_time_costs_no_full_redraw() {
    let mut s = setup();
    s.prompt_rows(6);
    let after_grow = s.tui.full_redraws();
    for rows in (1..6).rev() {
        s.prompt_rows(rows);
        assert_eq!(s.tui.full_redraws(), after_grow, "at {rows} rows");
        let screen = s.term.screen();
        let blanks = 6 - rows;
        assert!(
            screen[H - blanks..].iter().all(String::is_empty),
            "{screen:?}"
        );
        assert!(!screen[..H - blanks].contains(&String::new()), "{screen:?}");
    }
    let all = s.term.scroll_buffer();
    for i in 1..=H * 2 {
        assert_eq!(s.count(&format!("body {i}")), 1, "body {i}: {all:#?}");
    }
}

/// `terminal.clearOnShrink` still asks for a clean redraw on every shrink.
#[test]
fn clear_on_shrink_redraws_on_every_shrink() {
    let mut s = setup();
    s.tui.set_clear_on_shrink(true);
    s.prompt_rows(6);
    let before = s.tui.full_redraws();
    s.prompt_rows(5);
    s.prompt_rows(4);
    assert_eq!(s.tui.full_redraws(), before + 2);
}
