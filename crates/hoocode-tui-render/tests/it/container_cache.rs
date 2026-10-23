//! A container keeps the lines of children that report a render stamp, and
//! draws again only what changed.

use hoocode_tui_render::{fresh_stamp, Component, ComponentHandle, Container, Rendered};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A child with a stamp. `renders` counts how often it is drawn.
struct Stamped {
    text: String,
    stamp: u64,
    renders: Rc<Cell<usize>>,
}

impl Stamped {
    fn new(text: &str) -> (Rc<RefCell<Self>>, Rc<Cell<usize>>) {
        let renders = Rc::new(Cell::new(0));
        let this = Rc::new(RefCell::new(Self {
            text: text.to_string(),
            stamp: fresh_stamp(),
            renders: renders.clone(),
        }));
        (this, renders)
    }

    fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.stamp = fresh_stamp();
    }
}

impl Component for Stamped {
    fn render(&mut self, width: u16) -> Vec<String> {
        self.renders.set(self.renders.get() + 1);
        vec![format!("{}@{width}", self.text)]
    }

    fn render_stamp(&self) -> Option<u64> {
        Some(self.stamp)
    }
}

/// A child with no stamp: drawn every frame.
struct Plain(Rc<Cell<usize>>);

impl Component for Plain {
    fn render(&mut self, _width: u16) -> Vec<String> {
        self.0.set(self.0.get() + 1);
        vec!["plain".to_string()]
    }
}

/// Claims a stamp but changes its output without taking a new one.
struct Liar(Rc<Cell<usize>>);

impl Component for Liar {
    fn render(&mut self, _width: u16) -> Vec<String> {
        self.0.set(self.0.get() + 1);
        vec![format!("draw {}", self.0.get())]
    }

    fn render_stamp(&self) -> Option<u64> {
        Some(7)
    }
}

fn handle<C: Component + 'static>(c: &Rc<RefCell<C>>) -> ComponentHandle {
    c.clone()
}

// The checks below count renders, so their containers run with the staleness check off.
fn container() -> Container {
    let mut container = Container::new();
    container.set_cache_check(false);
    container
}

#[test]
fn an_unchanged_child_is_not_drawn_again() {
    let (a, a_renders) = Stamped::new("a");
    let (b, b_renders) = Stamped::new("b");
    let mut container = container();
    container.add_child(handle(&a));
    container.add_child(handle(&b));

    assert_eq!(container.render(10), vec!["a@10", "b@10"]);
    assert_eq!(container.render(10), vec!["a@10", "b@10"]);
    assert_eq!((a_renders.get(), b_renders.get()), (1, 1));

    b.borrow_mut().set("b2");
    assert_eq!(container.render(10), vec!["a@10", "b2@10"]);
    assert_eq!((a_renders.get(), b_renders.get()), (1, 2));
}

#[test]
fn a_new_width_draws_again() {
    let (a, renders) = Stamped::new("a");
    let mut container = container();
    container.add_child(handle(&a));
    container.render(10);
    assert_eq!(container.render(20), vec!["a@20"]);
    assert_eq!(renders.get(), 2);
}

#[test]
fn invalidate_draws_again() {
    let (a, renders) = Stamped::new("a");
    let mut container = container();
    container.add_child(handle(&a));
    container.render(10);
    container.invalidate();
    container.render(10);
    assert_eq!(renders.get(), 2);
}

#[test]
fn a_child_without_a_stamp_is_drawn_every_frame() {
    let counter = Rc::new(Cell::new(0));
    let plain = Rc::new(RefCell::new(Plain(counter.clone())));
    let mut container = container();
    container.add_child(handle(&plain));
    container.render(10);
    container.render(10);
    assert_eq!(counter.get(), 2);
}

#[test]
fn children_swapped_in_place_are_not_confused() {
    let (a, _) = Stamped::new("a");
    let (b, _) = Stamped::new("b");
    let mut container = container();
    container.add_child(handle(&a));
    container.render(10);
    container.children[0] = handle(&b);
    assert_eq!(container.render(10), vec!["b@10"]);
}

#[test]
fn row_offsets_follow_the_cached_children() {
    struct Tall(usize, u64);
    impl Component for Tall {
        fn render(&mut self, _width: u16) -> Vec<String> {
            vec!["x".to_string(); self.0]
        }
        fn render_stamp(&self) -> Option<u64> {
            Some(self.1)
        }
    }
    let mut container = container();
    for rows in [2, 3, 1] {
        container.add_child(Rc::new(RefCell::new(Tall(rows, fresh_stamp()))));
    }
    container.render(10);
    container.render(10);
    assert_eq!(container.child_row_offsets(10), Some(vec![0, 2, 5]));
}

#[test]
fn the_container_stamp_follows_its_children() {
    let (a, _) = Stamped::new("a");
    let (b, _) = Stamped::new("b");
    let mut container = container();
    container.add_child(handle(&a));
    container.add_child(handle(&b));
    let before = container.render_stamp().expect("all children stamped");
    assert_eq!(container.render_stamp(), Some(before));
    b.borrow_mut().set("b2");
    assert_ne!(container.render_stamp(), Some(before));
    container.add_child(handle(&Rc::new(RefCell::new(Plain(Rc::new(Cell::new(0)))))));
    assert_eq!(container.render_stamp(), None);
}

#[test]
fn rendered_hands_out_a_window_without_the_rest() {
    let (a, _) = Stamped::new("a");
    let tall: Rc<RefCell<dyn Component>> = {
        struct Rows(Vec<String>);
        impl Component for Rows {
            fn render(&mut self, _width: u16) -> Vec<String> {
                self.0.clone()
            }
        }
        Rc::new(RefCell::new(Rows(
            (0..10).map(|n| format!("r{n}")).collect(),
        )))
    };
    let mut container = container();
    container.add_child(handle(&a));
    container.add_child(tall);
    let mut rendered = Rendered::default();
    container.render_into(10, &mut rendered);
    assert_eq!(rendered.len(), 11);
    assert_eq!(rendered.window(0, 2), vec!["a@10", "r0"]);
    assert_eq!(rendered.window(9, 5), vec!["r8", "r9"]);
    assert_eq!(rendered.window(11, 5), Vec::<String>::new());
    assert_eq!(rendered.window(3, 0), Vec::<String>::new());
    assert_eq!(rendered.into_lines().len(), 11);
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "render cache is stale")]
fn a_stamp_that_misses_a_change_is_caught_in_debug_builds() {
    let liar = Rc::new(RefCell::new(Liar(Rc::new(Cell::new(0)))));
    let mut container = Container::new();
    container.add_child(handle(&liar));
    container.render(10);
    container.render(10);
}
