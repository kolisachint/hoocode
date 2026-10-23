//! A long stamped body in full-screen: the frame shows the right rows, and a
//! frame that changes only the tail draws only the tail.

use crate::support::*;
use hoocode_tui_render::{fresh_stamp, Component, ComponentHandle, Container, Tui};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const WIDTH: u16 = 30;
const HEIGHT: u16 = 12;
const ITEMS: usize = 300;

/// Two rows per item, counted when drawn, with a stamp.
struct Item {
    label: String,
    stamp: u64,
    draws: Rc<Cell<usize>>,
}

impl Component for Item {
    fn render(&mut self, _width: u16) -> Vec<String> {
        self.draws.set(self.draws.get() + 1);
        vec![format!("{} a", self.label), format!("{} b", self.label)]
    }

    fn render_stamp(&self) -> Option<u64> {
        Some(self.stamp)
    }
}

struct Setup {
    tui: Tui,
    term: Handle,
    items: Vec<Rc<RefCell<Item>>>,
    draws: Rc<Cell<usize>>,
}

fn setup() -> Setup {
    let (terminal, term) = virtual_terminal(WIDTH, HEIGHT);
    let mut tui = Tui::new(terminal, None);
    tui.set_fullscreen(true);
    tui.add_child(Lines::new(&["HUD"]));
    let draws = Rc::new(Cell::new(0));
    let body = Rc::new(RefCell::new(Container::new()));
    body.borrow_mut().set_cache_check(false);
    let mut items = Vec::new();
    for n in 0..ITEMS {
        let item = Rc::new(RefCell::new(Item {
            label: format!("item{n}"),
            stamp: fresh_stamp(),
            draws: draws.clone(),
        }));
        body.borrow_mut().add_child(item.clone());
        items.push(item);
    }
    let handle: ComponentHandle = body.clone();
    tui.add_child(handle.clone());
    tui.set_fullscreen_body(Some(handle));
    tui.add_child(Lines::new(&["prompt"]));
    let _events = tui.start();
    tui.request_render(true);
    Setup {
        tui,
        term,
        items,
        draws,
    }
}

#[test]
fn the_live_window_is_the_tail_of_a_long_body() {
    let setup = setup();
    let screen = setup.term.screen();
    assert_eq!(screen.len(), HEIGHT as usize);
    assert_eq!(screen[0], "HUD");
    // 10 body rows: the last five items.
    assert_eq!(screen[1], "item295 a");
    assert_eq!(screen[10], "item299 b");
    assert_eq!(screen[11], "prompt");
}

#[test]
fn a_frame_after_a_tail_change_draws_only_the_tail() {
    let mut setup = setup();
    let first = setup.draws.get();
    assert_eq!(first, ITEMS);
    {
        let mut tail = setup.items[ITEMS - 1].borrow_mut();
        tail.label = "tail".to_string();
        tail.stamp = fresh_stamp();
    }
    setup.tui.request_render(true);
    assert_eq!(setup.draws.get(), first + 1);
    assert_eq!(setup.term.screen()[10], "tail b");
}

#[test]
fn pinned_view_reads_rows_from_the_cache() {
    let mut setup = setup();
    assert!(setup.tui.scroll_by_lines(-100));
    setup.tui.request_render(true);
    let screen = setup.term.screen();
    // Pinned 100 rows above the tail, so rows near 490 of 600 show.
    assert!(
        screen.iter().any(|row| row.starts_with("item245 a")),
        "{screen:?}"
    );
    assert_eq!(setup.draws.get(), ITEMS, "no child was drawn again");
}
