//! Diagnostic: where the rows go when a short transcript meets the task panel.
//!
//! A fresh session on a tall screen shows blank rows between the last chat
//! message and the FLIGHT PLAN panel. This pins the row accounting for that
//! frame: the blanks are the body's top-aligned padding for a transcript
//! shorter than its viewport (spec: `docs/design/mission-control.md` item 2
//! under M1: "A short transcript starts at the top, and blank rows follow
//! it... The body is top-aligned"), not rows reserved by the panel. The
//! second test shows the recovery the other way: when the panel appears, the
//! body shrinks by exactly the panel's rows, so the panel takes its rows from
//! the former padding while the transcript stays whole.

use crate::support::*;
use hoocode_tui_render::{ComponentHandle, Container, Tui};

const WIDTH: u16 = 40;
const HEIGHT: u16 = 20;
const H: usize = HEIGHT as usize;

struct Setup {
    tui: Tui,
    term: Handle,
    panel: std::rc::Rc<std::cell::RefCell<Lines>>,
}

fn setup(panel_rows: &[&str]) -> Setup {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.set_fullscreen(true);
    tui.add_child(Lines::new(&["HUD"]));
    tui.add_child(Lines::new(&[""]));
    // Body: a 2-row header plus 5 chat rows, 7 transcript rows in all.
    let body = std::rc::Rc::new(std::cell::RefCell::new(Container::new()));
    body.borrow_mut().add_child(Lines::new(&["logo1", "logo2"]));
    body.borrow_mut()
        .add_child(Lines::new(&["msg1", "msg2", "msg3", "msg4", "msg5"]));
    let handle: ComponentHandle = body.clone();
    tui.add_child(handle.clone());
    tui.set_fullscreen_body(Some(handle));
    // Bottom: the task panel over a 2-row prompt.
    let panel = Lines::new(panel_rows);
    tui.add_child(panel.clone());
    tui.add_child(Lines::new(&["prompt1", "prompt2"]));
    let _events = tui.start();
    tui.request_render(true);
    Setup { tui, term, panel }
}

#[test]
fn short_transcript_pads_body_above_panel() {
    let setup = setup(&["TASKS", "todo1", "todo2", "todo3"]);
    let screen = setup.term.screen();
    assert_eq!(screen.len(), H);
    // Top chrome: HUD + gap.
    assert_eq!(screen[0], "HUD");
    assert_eq!(screen[1], "");
    // The whole transcript, top-aligned: header then chat.
    assert_eq!(
        screen[2..9].to_vec(),
        vec!["logo1", "logo2", "msg1", "msg2", "msg3", "msg4", "msg5"]
    );
    // Body keeps 12 rows (20 - 2 top - 6 bottom); 7 transcript rows leave 5 blank.
    assert!(
        screen[9..14].iter().all(|row| row.is_empty()),
        "body padding above the panel: {screen:?}"
    );
    // The bottom is contiguous: panel then prompt, no gap between them.
    assert_eq!(screen[14..18].to_vec(), vec!["TASKS", "todo1", "todo2", "todo3"]);
    assert_eq!(screen[18..20].to_vec(), vec!["prompt1", "prompt2"]);
}

#[test]
fn appearing_panel_takes_rows_from_padding_not_transcript() {
    let mut setup = setup(&[]);
    let before = setup.term.screen();
    // No panel: bottom is the 2-row prompt, body is 16 rows, 9 of them blank.
    assert_eq!(before[2..9].to_vec(), vec![
        "logo1", "logo2", "msg1", "msg2", "msg3", "msg4", "msg5"
    ]);
    assert!(before[9..18].iter().all(|row| row.is_empty()));
    assert_eq!(before[18..20].to_vec(), vec!["prompt1", "prompt2"]);

    // The task panel appears with 4 rows.
    setup.panel.borrow_mut().0 = vec![
        "TASKS".to_string(),
        "todo1".to_string(),
        "todo2".to_string(),
        "todo3".to_string(),
    ];
    setup.tui.request_render(true);
    let after = setup.term.screen();
    // The transcript is untouched and still whole; the padding shrank by
    // exactly the panel's 4 rows; panel and prompt stay contiguous.
    assert_eq!(after[2..9].to_vec(), vec![
        "logo1", "logo2", "msg1", "msg2", "msg3", "msg4", "msg5"
    ]);
    assert!(after[9..14].iter().all(|row| row.is_empty()));
    assert_eq!(after[14..18].to_vec(), vec!["TASKS", "todo1", "todo2", "todo3"]);
    assert_eq!(after[18..20].to_vec(), vec!["prompt1", "prompt2"]);
}
