//! Component tree primitives, ported from `tui.ts`'s `Component`/`Container`.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

static NEXT_STAMP: AtomicU64 = AtomicU64::new(1);

/// A render stamp no other component state has had: a counter, never reused.
/// Components keep one and take a new one whenever something that shows in
/// their output changes (see [`Component::render_stamp`]).
pub fn fresh_stamp() -> u64 {
    NEXT_STAMP.fetch_add(1, Ordering::Relaxed)
}

static RENDER_EPOCH: AtomicU64 = AtomicU64::new(1);

/// A change to something every component draws from but none owns (the
/// theme): lines a container kept before it are stale.
pub fn bump_render_epoch() {
    RENDER_EPOCH.fetch_add(1, Ordering::Relaxed);
}

/// The current render epoch, for caches that keep lines the way a [`Container`] does.
pub fn render_epoch() -> u64 {
    RENDER_EPOCH.load(Ordering::Relaxed)
}

/// One stamp standing for several, in order (FNV-1a over the words). A
/// composite component builds its stamp from its own and its children's.
pub fn mix_stamps(parts: impl IntoIterator<Item = u64>) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// Whether a cache hit is re-rendered and compared to the cached lines. On in
/// builds with debug assertions (tests included), so a component whose stamp
/// misses a change fails its tests; `HOOCODE_RENDER_CACHE_CHECK=0` turns it off.
fn verify_cache_hits() -> bool {
    static VERIFY: OnceLock<bool> = OnceLock::new();
    *VERIFY.get_or_init(|| {
        cfg!(debug_assertions)
            && std::env::var("HOOCODE_RENDER_CACHE_CHECK").map_or(true, |v| v != "0")
    })
}

/// A run of rendered rows: lines a component just drew, or lines a container
/// kept from an earlier frame and shares.
pub enum Piece {
    Owned(Vec<String>),
    Shared(Rc<[String]>),
}

impl Piece {
    pub fn lines(&self) -> &[String] {
        match self {
            Piece::Owned(lines) => lines,
            Piece::Shared(lines) => lines,
        }
    }
}

/// The rows of a render as pieces. Copying out a window of them
/// ([`window`](Self::window)) costs the window, not the whole output.
#[derive(Default)]
pub struct Rendered {
    pieces: Vec<Piece>,
    len: usize,
}

impl Rendered {
    pub fn push(&mut self, piece: Piece) {
        self.len += piece.lines().len();
        self.pieces.push(piece);
    }

    /// Total rows.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Rows `start..start + count` (fewer when the output ends first).
    pub fn window(&self, start: usize, count: usize) -> Vec<String> {
        let end = start.saturating_add(count).min(self.len);
        let mut out = Vec::with_capacity(end.saturating_sub(start));
        let mut offset = 0usize;
        for piece in &self.pieces {
            let lines = piece.lines();
            let piece_end = offset + lines.len();
            if piece_end > start && offset < end {
                let from = start.saturating_sub(offset);
                let to = end.min(piece_end) - offset;
                out.extend_from_slice(&lines[from..to]);
            }
            offset = piece_end;
            if offset >= end {
                break;
            }
        }
        out
    }

    /// All rows. Rows that were drawn this frame move; shared ones are copied.
    pub fn into_lines(self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.len);
        for piece in self.pieces {
            match piece {
                Piece::Owned(mut lines) => out.append(&mut lines),
                Piece::Shared(lines) => out.extend_from_slice(&lines),
            }
        }
        out
    }
}

/// A node in the TUI component tree.
///
/// The TypeScript original uses optional interface methods
/// (`handleInput?`, `wantsKeyRelease?`) and a `Focusable` type guard
/// (`"focused" in component`). Rust has no structural typing, so those
/// become default-implemented trait methods instead: `handle_input` is a
/// no-op by default, and `is_focusable`/`set_focused` replace the
/// `Focusable` interface.
pub trait Component {
    /// Render the component to lines for the given viewport width.
    fn render(&mut self, width: u16) -> Vec<String>;

    /// Handle keyboard input when the component has focus.
    fn handle_input(&mut self, _data: &str) {}

    /// Invalidate any cached rendering state (e.g. on theme change).
    fn invalidate(&mut self) {}

    /// A stamp for the component's output: while it returns the same
    /// `Some(stamp)` and the width is the same, `render` returns the same lines,
    /// and a [`Container`] may reuse the lines it kept. `None` (the default)
    /// means "unknown": the component is rendered every frame.
    ///
    /// Return `Some` only when the output is a function of state that changes
    /// through `&mut self` methods (each of which takes a [`fresh_stamp`]), of
    /// `invalidate` (theme changes), and of the width. A component that reads
    /// the clock, or state it does not own and cannot stamp, returns `None`.
    fn render_stamp(&self) -> Option<u64> {
        None
    }

    /// `render`, shared: what a [`Container`] keeps for a stamped child.
    fn render_shared(&mut self, width: u16) -> Rc<[String]> {
        self.render(width).into()
    }

    /// Whether the component answers [`render_into`](Self::render_into) itself
    /// and keeps its children's lines (a [`Container`]), so its parent takes
    /// its pieces as they are and does not keep a copy of all of them.
    fn is_container(&self) -> bool {
        false
    }

    /// `render`, appended to `out` as pieces: a container hands over the lines
    /// it kept for unchanged children without copying them.
    fn render_into(&mut self, width: u16, out: &mut Rendered) {
        out.push(Piece::Owned(self.render(width)));
    }

    /// Whether this component can receive focus and display a hardware cursor.
    fn is_focusable(&self) -> bool {
        false
    }

    /// Called by [`crate::Tui`] when focus changes, for focusable components.
    fn set_focused(&mut self, _focused: bool) {}

    /// The concrete component behind a handle, for components that opt in.
    /// TypeScript reaches into any object's fields; Rust code that needs the
    /// same (a caller inspecting a submenu a factory built) downcasts this.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }
}

pub type ComponentHandle = Rc<RefCell<dyn Component>>;

/// A component that contains other components, concatenating their
/// rendered lines.
///
/// Unlike the TypeScript original there is no reference-identity flatten
/// memoization (the terminal writer diffs by line *content*). What a container
/// keeps instead is each child's lines under the child's
/// [`Component::render_stamp`]: a child whose stamp and width are unchanged
/// since the last frame is not rendered again, and a window of the output
/// ([`Component::render_window`]) copies only the rows it asks for. A child
/// without a stamp is rendered every frame, as before.
#[derive(Default)]
pub struct Container {
    pub children: Vec<ComponentHandle>,
    /// Where each child's output started in the last render, and at what width.
    last_offsets: Option<(u16, Vec<usize>)>,
    /// Per child (by position): the lines it drew, the width and the stamp they are for.
    cache: Vec<Option<ChildCache>>,
    /// Overrides [`verify_cache_hits`] (tests that count renders turn it off).
    verify: Option<bool>,
}

struct ChildCache {
    width: u16,
    epoch: u64,
    stamp: u64,
    lines: Rc<[String]>,
}

impl Container {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_child(&mut self, component: ComponentHandle) {
        self.children.push(component);
    }

    pub fn remove_child(&mut self, component: &ComponentHandle) {
        self.children.retain(|c| !Rc::ptr_eq(c, component));
    }

    pub fn clear(&mut self) {
        self.children.clear();
    }

    /// Whether a cache hit is drawn again and compared with the kept lines
    /// (default: on in builds with debug assertions, see `verify_cache_hits`).
    pub fn set_cache_check(&mut self, on: bool) {
        self.verify = Some(on);
    }

    /// Where each direct child's output starts, in rows, from the last render
    /// (`childRowOffsets`). `None` before the first render or at a different
    /// width: "no answer", not "zero".
    pub fn child_row_offsets(&self, width: u16) -> Option<Vec<usize>> {
        match &self.last_offsets {
            Some((w, offsets)) if *w == width && offsets.len() == self.children.len() => {
                Some(offsets.clone())
            }
            _ => None,
        }
    }
}

impl Component for Container {
    fn render(&mut self, width: u16) -> Vec<String> {
        let mut rendered = Rendered::default();
        self.render_into(width, &mut rendered);
        rendered.into_lines()
    }

    fn render_into(&mut self, width: u16, out: &mut Rendered) {
        let child_count = self.children.len();
        self.cache.truncate(child_count);
        self.cache.resize_with(child_count, || None);
        let base = out.len();
        let mut offsets = Vec::with_capacity(child_count);
        let epoch = render_epoch();
        for (index, child) in self.children.iter().enumerate() {
            offsets.push(out.len() - base);
            if child.borrow().is_container() {
                self.cache[index] = None;
                child.borrow_mut().render_into(width, out);
                continue;
            }
            let Some(stamp) = child.borrow().render_stamp() else {
                self.cache[index] = None;
                out.push(Piece::Owned(child.borrow_mut().render(width)));
                continue;
            };
            let hit = self.cache[index]
                .as_ref()
                .filter(|entry| {
                    entry.width == width && entry.stamp == stamp && entry.epoch == epoch
                })
                .map(|entry| entry.lines.clone());
            let lines = match hit {
                Some(lines) => {
                    if self.verify.unwrap_or_else(verify_cache_hits) {
                        let fresh = child.borrow_mut().render(width);
                        assert!(
                            fresh[..] == lines[..],
                            "render cache is stale: a child kept stamp {stamp} at width {width} \
                             but its output changed (child {index} of {child_count})"
                        );
                    }
                    lines
                }
                None => {
                    let lines = child.borrow_mut().render_shared(width);
                    // A render that changed the stamp (it settled something
                    // lazily) is not kept; the next frame is stable.
                    let after = child.borrow().render_stamp();
                    self.cache[index] = (after == Some(stamp)).then(|| ChildCache {
                        width,
                        epoch,
                        stamp,
                        lines: lines.clone(),
                    });
                    lines
                }
            };
            out.push(Piece::Shared(lines));
        }
        self.last_offsets = Some((width, offsets));
    }

    fn is_container(&self) -> bool {
        true
    }

    /// The children's stamps, in order, when every child has one.
    fn render_stamp(&self) -> Option<u64> {
        let mut parts = Vec::with_capacity(self.children.len() + 1);
        parts.push(self.children.len() as u64);
        for child in &self.children {
            parts.push(child.borrow().render_stamp()?);
        }
        Some(mix_stamps(parts))
    }

    fn invalidate(&mut self) {
        self.cache.clear();
        for child in &self.children {
            child.borrow_mut().invalidate();
        }
    }
}

/// A spacer whose height the renderer decides, so the layout can fill the
/// screen (`FlexSpacer`, from `components/spacer.ts`; it lives here because
/// the root sizes it). Put it between the part that flows from the top and
/// the chrome that hangs off the bottom, and hand it to
/// [`crate::Tui::set_flex_spacer`].
#[derive(Default)]
pub struct FlexSpacer {
    height: usize,
}

impl FlexSpacer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rows it is currently contributing.
    pub fn current_height(&self) -> usize {
        self.height
    }

    /// Set the fill; true when it changed and a re-flatten is owed.
    pub fn set_height(&mut self, height: i64) -> bool {
        let next = height.max(0) as usize;
        if next == self.height {
            return false;
        }
        self.height = next;
        true
    }
}

impl Component for FlexSpacer {
    fn render(&mut self, _width: u16) -> Vec<String> {
        vec![String::new(); self.height]
    }
}

/// A child that can be taken off screen (`Slot`). While hidden the child is
/// not rendered at all, so an animating child is paused, not merely hidden.
/// Swapping the occupant keeps the slot's place in the tree.
pub struct Slot {
    component: ComponentHandle,
    hidden: bool,
}

impl Slot {
    pub fn new(component: ComponentHandle) -> Self {
        Self {
            component,
            hidden: false,
        }
    }

    /// Whoever is in the slot right now.
    pub fn child(&self) -> ComponentHandle {
        self.component.clone()
    }

    pub fn visible(&self) -> bool {
        !self.hidden
    }

    /// Returns whether this changed anything, so callers can skip a render.
    pub fn set_visible(&mut self, visible: bool) -> bool {
        if self.hidden != visible {
            return false;
        }
        self.hidden = !visible;
        true
    }
}

impl Component for Slot {
    fn render(&mut self, width: u16) -> Vec<String> {
        if self.hidden {
            return Vec::new();
        }
        self.component.borrow_mut().render(width)
    }

    fn invalidate(&mut self) {
        self.component.borrow_mut().invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counted {
        stamp: u64,
        renders: Rc<std::cell::Cell<usize>>,
    }

    impl Component for Counted {
        fn render(&mut self, _width: u16) -> Vec<String> {
            self.renders.set(self.renders.get() + 1);
            vec!["row".to_string()]
        }

        fn render_stamp(&self) -> Option<u64> {
            Some(self.stamp)
        }
    }

    /// The epoch is process-wide, so this test lives in the library's own test
    /// binary, where no other test counts renders.
    #[test]
    fn a_render_epoch_bump_draws_again() {
        let renders = Rc::new(std::cell::Cell::new(0));
        let child = Rc::new(RefCell::new(Counted {
            stamp: fresh_stamp(),
            renders: renders.clone(),
        }));
        let mut container = Container::new();
        container.set_cache_check(false);
        container.add_child(child);
        container.render(10);
        container.render(10);
        assert_eq!(renders.get(), 1);
        bump_render_epoch();
        container.render(10);
        assert_eq!(renders.get(), 2);
    }
}
